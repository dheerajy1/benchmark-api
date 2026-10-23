use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use reqwest::{blocking::Client, redirect::Policy};

use crate::features::v1::benchmark::{
    live::{RunControl, gauges::RequestCounters},
    stop::Stop,
};

use super::{
    observation::RequestObservation,
    plan::{WorkloadLimit, WorkloadPlan},
};

// How often the caller thread checks that the target is still alive
const MONITOR_INTERVAL: Duration = Duration::from_millis(25);

pub struct WorkloadOutcome {
    pub observations: Vec<RequestObservation>,
    // Actual elapsed workload duration, including in-flight requests finishing
    pub elapsed: Duration,
    // True when the target process was gone during or right after the workload
    pub target_exited: bool,
}

// Shared state the workers use to decide whether to issue another request
struct WorkerLimits {
    next_request: AtomicU64,
    cancel: AtomicBool,
    // Only set in duration mode
    deadline: Option<Instant>,
}

// Live observation of a running workload: the workers count their finished requests, and
// the caller thread calls `on_tick` with the elapsed workload time every `interval`
pub struct Observer<'a> {
    pub counters: &'a RequestCounters,
    pub interval: Duration,
    pub on_tick: &'a mut dyn FnMut(Duration),
}

// Runs the workload with at most plan.concurrency requests in flight
pub fn execute<F>(plan: &WorkloadPlan, is_alive: F) -> Result<WorkloadOutcome, String>
where
    F: FnMut() -> bool,
{
    execute_observed(plan, is_alive, None)
}

// Same workload, optionally reporting its progress while it runs. Without an observer
// nothing is counted and no tick is made.
pub fn execute_observed<F>(
    plan: &WorkloadPlan,
    is_alive: F,
    observer: Option<Observer<'_>>,
) -> Result<WorkloadOutcome, String>
where
    F: FnMut() -> bool,
{
    execute_cancellable(plan, is_alive, observer, &RunControl::default(), || {})
        .map_err(Stop::into_message)
}

// Same workload, stopped by a cancel request. The workers stop issuing requests, `on_cancel`
// is called once so the caller can kill the target and make the requests in flight fail at
// once, and the workers are joined before this answers.
pub fn execute_cancellable<F, C>(
    plan: &WorkloadPlan,
    mut is_alive: F,
    mut observer: Option<Observer<'_>>,
    control: &RunControl,
    mut on_cancel: C,
) -> Result<WorkloadOutcome, Stop>
where
    F: FnMut() -> bool,
    C: FnMut(),
{
    let client = build_client(plan)?;
    let mut cancelled = false;
    let counters = observer.as_ref().map(|observer| observer.counters);
    let mut target_exited = false;

    let started = Instant::now();

    let limits = WorkerLimits {
        next_request: AtomicU64::new(0),
        cancel: AtomicBool::new(false),
        deadline: match plan.limit {
            WorkloadLimit::Requests(_) => None,
            WorkloadLimit::Duration(duration) => Some(started + duration),
        },
    };

    let observations = thread::scope(|scope| -> Result<Vec<RequestObservation>, String> {
        let mut workers = Vec::with_capacity(plan.concurrency);

        for index in 0..plan.concurrency {
            let spawned = thread::Builder::new()
                .name(format!("workload-worker-{index}"))
                .spawn_scoped(scope, || run_worker(&client, plan, &limits, counters));

            match spawned {
                Ok(handle) => workers.push(handle),
                Err(error) => {
                    limits.cancel.store(true, Ordering::SeqCst);
                    return Err(format!("Failed to spawn workload worker: {error}"));
                }
            }
        }

        let mut next_tick = observer
            .as_ref()
            .map(|observer| started + observer.interval);

        // Caller thread watches the target while workers send requests
        loop {
            if workers.iter().all(|worker| worker.is_finished()) {
                break;
            }

            if let (Some(observer), Some(due)) = (observer.as_mut(), next_tick) {
                let now = Instant::now();

                if now >= due {
                    (observer.on_tick)(now.duration_since(started));

                    next_tick = Some(due + observer.interval);
                }
            }

            if control.is_cancel_requested() {
                cancelled = true;
                limits.cancel.store(true, Ordering::SeqCst);

                // Killing the target first makes the requests in flight fail at once
                on_cancel();

                break;
            }

            if !is_alive() {
                target_exited = true;
                limits.cancel.store(true, Ordering::SeqCst);
                break;
            }

            thread::sleep(MONITOR_INTERVAL);
        }

        let mut observations = Vec::new();

        for worker in workers {
            match worker.join() {
                Ok(mut collected) => observations.append(&mut collected),
                Err(_) => return Err("Workload worker thread panicked".to_string()),
            }
        }

        Ok(observations)
    })
    .map_err(Stop::Failed)?;

    if cancelled {
        return Err(Stop::Cancelled);
    }

    let elapsed = started.elapsed();

    // Requests may all finish (as failures) before the monitor notices a dead target
    if !target_exited && !is_alive() {
        target_exited = true;
    }

    Ok(WorkloadOutcome {
        observations,
        elapsed,
        target_exited,
    })
}

fn build_client(plan: &WorkloadPlan) -> Result<Client, String> {
    Client::builder()
        .timeout(plan.request_timeout)
        .redirect(Policy::none())
        .no_proxy()
        .pool_max_idle_per_host(plan.concurrency)
        .build()
        .map_err(|error| format!("Failed to build HTTP client: {error}"))
}

fn run_worker(
    client: &Client,
    plan: &WorkloadPlan,
    limits: &WorkerLimits,
    counters: Option<&RequestCounters>,
) -> Vec<RequestObservation> {
    let mut observations = Vec::new();

    while may_issue_request(plan, limits) {
        let observation = send_request(client, plan);

        if let Some(counters) = counters {
            counters.record(observation.success);
        }

        observations.push(observation);
    }

    observations
}

// Request-count mode claims a request number, duration mode checks the deadline
fn may_issue_request(plan: &WorkloadPlan, limits: &WorkerLimits) -> bool {
    if limits.cancel.load(Ordering::SeqCst) {
        return false;
    }

    match plan.limit {
        WorkloadLimit::Requests(total) => {
            limits.next_request.fetch_add(1, Ordering::SeqCst) < total
        }
        WorkloadLimit::Duration(_) => limits
            .deadline
            .is_some_and(|deadline| Instant::now() < deadline),
    }
}

// Times one request from just before sending until the body is fully read or it fails
fn send_request(client: &Client, plan: &WorkloadPlan) -> RequestObservation {
    let started = Instant::now();

    let result = client
        .get(plan.url.as_str())
        .headers(plan.headers.clone())
        .send();

    let success = match result {
        Ok(response) => {
            let status_ok = response.status().is_success();
            let body_ok = response.bytes().is_ok();

            status_ok && body_ok
        }
        Err(_) => false,
    };

    RequestObservation {
        duration: started.elapsed(),
        success,
    }
}
