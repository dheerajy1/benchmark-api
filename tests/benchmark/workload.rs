use std::{collections::BTreeMap, net::TcpListener, time::Duration};

use benchmark_api::features::v1::benchmark::{
    request::WorkloadRequest,
    workload::{
        executor,
        plan::{DEFAULT_REQUEST_TIMEOUT, MAX_DURATION_SECONDS, WorkloadLimit, WorkloadPlan},
    },
};

use super::test_server::{Behavior, TestServer};

const TIMEOUT: Duration = Duration::from_secs(5);

fn workload(path: &str, requests: u64, concurrency: u64) -> WorkloadRequest {
    WorkloadRequest {
        method: "GET".to_string(),
        path: path.to_string(),
        headers: BTreeMap::new(),
        requests: Some(requests),
        duration_seconds: None,
        concurrency,
    }
}

fn duration_workload(path: &str, seconds: u64, concurrency: u64) -> WorkloadRequest {
    WorkloadRequest {
        requests: None,
        duration_seconds: Some(seconds),
        ..workload(path, 1, concurrency)
    }
}

fn plan_for(port: u16, request: &WorkloadRequest, timeout: Duration) -> WorkloadPlan {
    WorkloadPlan::new(request, "127.0.0.1", port, timeout).unwrap()
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();

    listener.local_addr().unwrap().port()
}

#[test]
fn default_request_timeout_is_ten_seconds() {
    assert_eq!(DEFAULT_REQUEST_TIMEOUT, Duration::from_secs(10));
}

#[test]
fn plan_builds_url_and_keeps_counts() {
    let plan = plan_for(8090, &workload("/api/v1/health/app", 1000, 50), TIMEOUT);

    assert_eq!(plan.url, "http://127.0.0.1:8090/api/v1/health/app");
    assert_eq!(plan.limit, WorkloadLimit::Requests(1000));
    assert_eq!(plan.concurrency, 50);
    assert_eq!(plan.request_timeout, TIMEOUT);
}

#[test]
fn plan_limits_concurrency_to_request_count() {
    let plan = plan_for(8090, &workload("/", 10, 50), TIMEOUT);

    assert_eq!(plan.limit, WorkloadLimit::Requests(10));
    assert_eq!(plan.concurrency, 10);
}

#[test]
fn plan_brackets_ipv6_hosts() {
    let plan = WorkloadPlan::new(&workload("/health", 1, 1), "::1", 8090, TIMEOUT).unwrap();

    assert_eq!(plan.url, "http://[::1]:8090/health");
}

#[test]
fn plan_rejects_zero_counts_and_bad_input() {
    assert!(WorkloadPlan::new(&workload("/", 0, 1), "127.0.0.1", 1, TIMEOUT).is_err());
    assert!(WorkloadPlan::new(&workload("/", 1, 0), "127.0.0.1", 1, TIMEOUT).is_err());
    assert!(WorkloadPlan::new(&workload("no-slash", 1, 1), "127.0.0.1", 1, TIMEOUT).is_err());

    let mut post = workload("/", 1, 1);

    post.method = "POST".to_string();

    assert!(WorkloadPlan::new(&post, "127.0.0.1", 1, TIMEOUT).is_err());
}

#[test]
fn executes_exact_total_request_count() {
    let server = TestServer::start(Behavior::status(200));
    let plan = plan_for(server.port, &workload("/ok", 200, 10), TIMEOUT);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert_eq!(outcome.observations.len(), 200);
    assert_eq!(server.total_requests(), 200);
    assert!(!outcome.target_exited);
}

#[test]
fn executes_exact_count_when_concurrency_exceeds_requests() {
    let server = TestServer::start(Behavior::status(200));
    let plan = plan_for(server.port, &workload("/ok", 7, 50), TIMEOUT);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert_eq!(outcome.observations.len(), 7);
    assert_eq!(server.total_requests(), 7);
}

#[test]
fn never_exceeds_configured_concurrency() {
    let server = TestServer::start(Behavior::delayed(200, Duration::from_millis(40)));
    let plan = plan_for(server.port, &workload("/slow", 40, 5), TIMEOUT);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert_eq!(outcome.observations.len(), 40);
    assert!(server.max_in_flight() <= 5);
    assert!(server.max_in_flight() >= 2);
}

#[test]
fn concurrency_of_one_runs_requests_sequentially() {
    let server = TestServer::start(Behavior::delayed(200, Duration::from_millis(10)));
    let plan = plan_for(server.port, &workload("/slow", 10, 1), TIMEOUT);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert_eq!(outcome.observations.len(), 10);
    assert_eq!(server.max_in_flight(), 1);
}

#[test]
fn counts_2xx_responses_as_successful() {
    let server = TestServer::start(Behavior::status(200));
    let plan = plan_for(server.port, &workload("/ok", 20, 4), TIMEOUT);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert!(outcome.observations.iter().all(|item| item.success));
}

#[test]
fn counts_non_2xx_responses_as_failed() {
    for status in [404, 500, 302] {
        let server = TestServer::start(Behavior::status(status));
        let plan = plan_for(server.port, &workload("/bad", 10, 2), TIMEOUT);

        let outcome = executor::execute(&plan, || true).unwrap();

        assert_eq!(outcome.observations.len(), 10);
        assert!(
            outcome.observations.iter().all(|item| !item.success),
            "status {status} should be a failed request"
        );
    }
}

#[test]
fn connection_failures_are_failed_requests() {
    let plan = plan_for(closed_port(), &workload("/ok", 5, 2), TIMEOUT);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert_eq!(outcome.observations.len(), 5);
    assert!(outcome.observations.iter().all(|item| !item.success));
}

#[test]
fn per_request_timeout_fails_hanging_requests() {
    let server = TestServer::start(Behavior::Hang);
    let plan = plan_for(
        server.port,
        &workload("/hang", 2, 2),
        Duration::from_millis(200),
    );

    let outcome = executor::execute(&plan, || true).unwrap();

    assert_eq!(outcome.observations.len(), 2);
    assert!(outcome.observations.iter().all(|item| !item.success));
    assert!(
        outcome
            .observations
            .iter()
            .all(|item| item.duration >= Duration::from_millis(150))
    );
    assert!(outcome.elapsed < Duration::from_secs(3));
}

#[test]
fn sends_configured_headers() {
    let server = TestServer::start(Behavior::status(200));

    let mut request = workload("/with-headers", 3, 1);

    request
        .headers
        .insert("X-Client-Id".to_string(), "client-abc".to_string());
    request
        .headers
        .insert("X-Client-Secret".to_string(), "secret-xyz".to_string());

    let plan = plan_for(server.port, &request, TIMEOUT);

    executor::execute(&plan, || true).unwrap();

    let heads = server.heads();

    assert_eq!(heads.len(), 3);

    for head in heads {
        assert!(head.starts_with("get /with-headers http/1.1"));
        assert!(head.contains("x-client-id: client-abc"));
        assert!(head.contains("x-client-secret: secret-xyz"));
    }
}

#[test]
fn measures_per_request_latency_and_elapsed_time() {
    let server = TestServer::start(Behavior::delayed(200, Duration::from_millis(50)));
    let plan = plan_for(server.port, &workload("/slow", 4, 2), TIMEOUT);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert!(
        outcome
            .observations
            .iter()
            .all(|item| item.duration >= Duration::from_millis(50))
    );
    assert!(
        outcome
            .observations
            .iter()
            .all(|item| item.duration < Duration::from_secs(3))
    );
    assert!(outcome.elapsed >= Duration::from_millis(100));
}

#[test]
fn stops_issuing_requests_when_the_target_exits() {
    let server = TestServer::start(Behavior::delayed(200, Duration::from_millis(100)));
    let plan = plan_for(server.port, &workload("/slow", 1000, 2), TIMEOUT);

    let mut checks = 0;

    let outcome = executor::execute(&plan, || {
        checks += 1;
        checks <= 2
    })
    .unwrap();

    assert!(outcome.target_exited);
    assert!(!outcome.observations.is_empty());
    assert!(outcome.observations.len() < 1000);
}

#[test]
fn reports_target_exit_noticed_after_the_workload() {
    let server = TestServer::start(Behavior::status(200));
    let plan = plan_for(server.port, &workload("/ok", 5, 1), TIMEOUT);

    let outcome = executor::execute(&plan, || false).unwrap();

    assert!(outcome.target_exited);
}

#[test]
fn duration_plan_keeps_the_full_concurrency() {
    let plan = plan_for(8090, &duration_workload("/", 30, 50), TIMEOUT);

    assert_eq!(plan.limit, WorkloadLimit::Duration(Duration::from_secs(30)));
    assert_eq!(plan.concurrency, 50);
}

#[test]
fn plan_enforces_the_duration_rules() {
    let build = |request: &WorkloadRequest| WorkloadPlan::new(request, "127.0.0.1", 1, TIMEOUT);

    assert!(build(&duration_workload("/", 1, 1)).is_ok());
    assert!(build(&duration_workload("/", MAX_DURATION_SECONDS, 1)).is_ok());
    assert!(build(&duration_workload("/", 0, 1)).is_err());
    assert!(build(&duration_workload("/", MAX_DURATION_SECONDS + 1, 1)).is_err());
    assert!(build(&duration_workload("/", 5, 0)).is_err());

    let mut both = workload("/", 10, 1);

    both.duration_seconds = Some(5);

    assert!(build(&both).is_err());

    let mut neither = workload("/", 10, 1);

    neither.requests = None;

    assert!(build(&neither).is_err());
}

#[test]
fn maximum_duration_is_three_hundred_seconds() {
    assert_eq!(MAX_DURATION_SECONDS, 300);
}
