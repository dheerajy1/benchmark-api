use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use benchmark_api::features::v1::benchmark::{
    metrics::{
        http_latency::{self, round_two_decimals},
        memory::{self, MemoryConfig},
    },
    request::WorkloadRequest,
    workload::{
        executor,
        plan::{WorkloadLimit, WorkloadPlan},
    },
};

use super::test_server::{Behavior, TestServer};

const TIMEOUT: Duration = Duration::from_secs(5);

fn duration_request(seconds: u64, concurrency: u64) -> WorkloadRequest {
    WorkloadRequest {
        method: "GET".to_string(),
        path: "/ok".to_string(),
        headers: BTreeMap::new(),
        requests: None,
        duration_seconds: Some(seconds),
        concurrency,
    }
}

// Request-level duration is whole seconds, so fast tests shorten the plan afterwards
fn short_plan(port: u16, millis: u64, concurrency: u64) -> WorkloadPlan {
    let mut plan = WorkloadPlan::new(
        &duration_request(1, concurrency),
        "127.0.0.1",
        port,
        TIMEOUT,
    )
    .unwrap();

    plan.limit = WorkloadLimit::Duration(Duration::from_millis(millis));

    plan
}

#[test]
fn runs_for_the_requested_period() {
    let server = TestServer::start(Behavior::status(200));

    let plan =
        WorkloadPlan::new(&duration_request(1, 4), "127.0.0.1", server.port, TIMEOUT).unwrap();

    let outcome = executor::execute(&plan, || true).unwrap();

    assert!(outcome.elapsed >= Duration::from_secs(1));
    assert!(outcome.elapsed < Duration::from_secs(3));
    assert!(!outcome.observations.is_empty());
    assert!(!outcome.target_exited);
}

#[test]
fn keeps_issuing_requests_until_the_deadline() {
    let server = TestServer::start(Behavior::status(200));
    let plan = short_plan(server.port, 400, 2);

    let outcome = executor::execute(&plan, || true).unwrap();

    // Far more than a handful of requests fit into the period
    assert!(outcome.observations.len() > 20);
    assert!(outcome.observations.iter().all(|item| item.success));
}

#[test]
fn stops_issuing_requests_after_the_deadline() {
    let server = TestServer::start(Behavior::status(200));
    let plan = short_plan(server.port, 300, 4);

    let outcome = executor::execute(&plan, || true).unwrap();

    thread::sleep(Duration::from_millis(150));

    assert_eq!(server.total_requests(), outcome.observations.len());
}

#[test]
fn concurrency_limit_holds_in_duration_mode() {
    let server = TestServer::start(Behavior::delayed(200, Duration::from_millis(40)));
    let plan = short_plan(server.port, 400, 5);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert!(!outcome.observations.is_empty());
    assert!(server.max_in_flight() <= 5);
    assert!(server.max_in_flight() >= 2);
}

#[test]
fn concurrency_of_one_runs_sequentially_in_duration_mode() {
    let server = TestServer::start(Behavior::delayed(200, Duration::from_millis(20)));
    let plan = short_plan(server.port, 300, 1);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert!(!outcome.observations.is_empty());
    assert_eq!(server.max_in_flight(), 1);
}

#[test]
fn statistics_come_from_observed_request_durations() {
    let server = TestServer::start(Behavior::delayed(200, Duration::from_millis(20)));
    let plan = short_plan(server.port, 300, 2);

    let outcome = executor::execute(&plan, || true).unwrap();

    let result = http_latency::summarize(&outcome.observations, outcome.elapsed, false);

    assert_eq!(result.status, "completed");
    assert_eq!(result.total_requests, outcome.observations.len() as u64);
    assert_eq!(result.successful_requests, result.total_requests);
    assert_eq!(result.failed_requests, 0);
    assert_eq!(result.duration_ms, outcome.elapsed.as_millis() as u64);

    let min = result.min_ms.unwrap();
    let p50 = result.p50_ms.unwrap();
    let p95 = result.p95_ms.unwrap();
    let p99 = result.p99_ms.unwrap();
    let max = result.max_ms.unwrap();
    let mean = result.mean_ms.unwrap();

    assert!(min >= 20.0, "min was {min}");
    assert!(min <= p50 && p50 <= p95 && p95 <= p99 && p99 <= max);
    assert!(mean >= min && mean <= max);
}

#[test]
fn throughput_uses_the_actual_elapsed_time_and_attempted_requests() {
    let server = TestServer::start(Behavior::status(200));
    let plan = short_plan(server.port, 300, 4);

    let outcome = executor::execute(&plan, || true).unwrap();

    let result = http_latency::summarize(&outcome.observations, outcome.elapsed, false);

    let expected =
        round_two_decimals(outcome.observations.len() as f64 / outcome.elapsed.as_secs_f64());

    assert_eq!(result.requests_per_second, expected);
    assert!(result.requests_per_second > 0.0);
}

#[test]
fn non_2xx_responses_are_failed_requests_in_duration_mode() {
    let server = TestServer::start(Behavior::status(500));
    let plan = short_plan(server.port, 200, 2);

    let outcome = executor::execute(&plan, || true).unwrap();

    let result = http_latency::summarize(&outcome.observations, outcome.elapsed, false);

    assert!(result.total_requests > 0);
    assert_eq!(result.successful_requests, 0);
    assert_eq!(result.failed_requests, result.total_requests);
}

#[test]
fn target_exit_ends_a_duration_workload_early() {
    let server = TestServer::start(Behavior::delayed(200, Duration::from_millis(100)));

    let mut plan = short_plan(server.port, 1000, 2);

    plan.limit = WorkloadLimit::Duration(Duration::from_secs(30));

    let mut checks = 0;

    let outcome = executor::execute(&plan, || {
        checks += 1;
        checks <= 2
    })
    .unwrap();

    let result = http_latency::summarize(
        &outcome.observations,
        outcome.elapsed,
        outcome.target_exited,
    );

    assert!(outcome.target_exited);
    assert!(outcome.elapsed < Duration::from_secs(5));
    assert!(!outcome.observations.is_empty());
    assert_eq!(result.status, "target_exited");
}

#[test]
fn memory_sampling_covers_the_whole_duration() {
    let server = TestServer::start(Behavior::status(200));
    let plan = short_plan(server.port, 500, 2);

    let started = Instant::now();
    let calls = Arc::new(AtomicU64::new(0));
    let reader_calls = Arc::clone(&calls);

    let config = MemoryConfig {
        stabilization: Duration::from_millis(0),
        sample_interval: Duration::from_millis(20),
    };

    // RSS only grows late in the workload, the peak must still catch it
    let session = memory::begin(&config, move || {
        reader_calls.fetch_add(1, Ordering::SeqCst);

        Ok(if started.elapsed() >= Duration::from_millis(400) {
            8000
        } else {
            1000
        })
    })
    .unwrap();

    let outcome = executor::execute(&plan, || true).unwrap();
    let finished = session.finish().unwrap();

    assert!(outcome.elapsed >= Duration::from_millis(500));
    assert!(
        finished.samples >= 10,
        "only {} samples during a 500 ms workload",
        finished.samples
    );
    assert_eq!(finished.memory.baseline.rss_bytes, 1000);
    assert_eq!(finished.memory.peak.rss_bytes, 8000);
    assert_eq!(finished.memory.delta.rss_bytes, 7000);
}
