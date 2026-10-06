use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use reqwest::{blocking::Client, redirect::Policy};

use super::{observation::RequestObservation, plan::WorkloadPlan};

// How often the caller thread checks that the target is still alive
const MONITOR_INTERVAL: Duration = Duration::from_millis(25);

pub struct WorkloadOutcome {
    pub observations: Vec<RequestObservation>,
    // Total elapsed workload duration
    pub elapsed: Duration,
    // True when the target process was gone during or right after the workload
    pub target_exited: bool,
}

// Runs the workload with at most plan.concurrency requests in flight
pub fn execute<F>(plan: &WorkloadPlan, mut is_alive: F) -> Result<WorkloadOutcome, String>
where
    F: FnMut() -> bool,
{
    let client = build_client(plan)?;
    let next_request = AtomicU64::new(0);
    let cancel = AtomicBool::new(false);
    let mut target_exited = false;

    let started = Instant::now();

    let observations = thread::scope(|scope| -> Result<Vec<RequestObservation>, String> {
        let mut workers = Vec::with_capacity(plan.concurrency);

        for index in 0..plan.concurrency {
            let spawned = thread::Builder::new()
                .name(format!("workload-worker-{index}"))
                .spawn_scoped(scope, || run_worker(&client, plan, &next_request, &cancel));

            match spawned {
                Ok(handle) => workers.push(handle),
                Err(error) => {
                    cancel.store(true, Ordering::SeqCst);
                    return Err(format!("Failed to spawn workload worker: {error}"));
                }
            }
        }

        // Caller thread watches the target while workers send requests
        loop {
            if workers.iter().all(|worker| worker.is_finished()) {
                break;
            }

            if !is_alive() {
                target_exited = true;
                cancel.store(true, Ordering::SeqCst);
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
    })?;

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
    next_request: &AtomicU64,
    cancel: &AtomicBool,
) -> Vec<RequestObservation> {
    let mut observations = Vec::new();

    loop {
        if cancel.load(Ordering::SeqCst) {
            break;
        }

        let index = next_request.fetch_add(1, Ordering::SeqCst);

        if index >= plan.requests {
            break;
        }

        observations.push(send_request(client, plan));
    }

    observations
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
