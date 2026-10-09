use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use benchmark_api::{
    app, config::env::Env, features::v1::benchmark::model::BenchmarkResult,
    persistence::repository::BenchmarkRepository,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{
    comparison_sample::{result, skipped},
    fake_repository::{FailingRepository, InMemoryRepository},
};

const URI: &str = "/api/v1/benchmarks/overview";

fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

async fn send(
    repository: Arc<dyn BenchmarkRepository>,
    method: Method,
    uri: &str,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap();

    let response = app::router(test_env(), repository)
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn get(repository: &Arc<InMemoryRepository>, uri: &str) -> (StatusCode, Value) {
    send(repository.clone(), Method::GET, uri).await
}

fn assert_error_shape(body: &Value, http_code: u64, code: &str) {
    assert_eq!(body["success"], json!(false));
    assert_eq!(body["httpCode"], json!(http_code));
    assert_eq!(body["code"], json!(code));
    assert!(body["message"].is_string());
    assert!(body["error"].is_string());
}

// A UUIDv7 with an exact millisecond timestamp, tail only makes the id unique
fn v7_at(milliseconds: u64, tail: u64) -> Uuid {
    let value = (u128::from(milliseconds) << 80)
        | (0x7u128 << 76)
        | (0b10u128 << 62)
        | u128::from(tail & ((1 << 62) - 1));

    Uuid::from_u128(value)
}

// The id of run number `number`, one second after the previous number
fn id(number: u64) -> Uuid {
    v7_at(number * 1000, number)
}

// Seeds `count` complete runs one second apart, the newest has the highest number
fn seed(repository: &InMemoryRepository, count: u64) -> Vec<Uuid> {
    (1..=count)
        .map(|number| {
            repository.seed_result(result(id(number)));

            id(number)
        })
        .collect()
}

// Seeds one run and returns the overview data of it
async fn overview_of(run: BenchmarkResult) -> Value {
    let repository = Arc::new(InMemoryRepository::default());

    repository.seed_result(run);

    let (status, body) = get(&repository, URI).await;

    assert_eq!(status, StatusCode::OK);

    body["data"].clone()
}

// The trend point of a run with the given six metric values
fn point(run_id: Uuid, metrics: [Value; 6]) -> Value {
    let [build, startup, memory, cpu, rps, p95] = metrics;

    json!({
        "run_id": run_id.to_string(),
        "created_at": "1970-01-01T00:00:01.000Z",
        "build_time_ms": build,
        "startup_time_ms": startup,
        "memory_peak_bytes": memory,
        "cpu_average_percent": cpu,
        "requests_per_second": rps,
        "p95_ms": p95
    })
}

fn trend_ids(body: &Value) -> Vec<String> {
    body["data"]["trends"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["run_id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn an_empty_database_returns_zero_runs_no_latest_and_no_trends() {
    let repository = Arc::new(InMemoryRepository::default());

    let (status, body) = get(&repository, URI).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["httpCode"], json!(200));
    assert_eq!(body["message"], json!("Benchmark overview retrieved."));
    assert_eq!(
        body["data"],
        json!({
            "totals": { "total_runs": 0 },
            "latest": null,
            "trends": []
        })
    );
}

#[tokio::test]
async fn one_complete_run_fills_totals_latest_and_trends() {
    let data = overview_of(result(id(1))).await;

    assert_eq!(data["totals"], json!({ "total_runs": 1 }));
    assert_eq!(
        data["latest"],
        json!({
            "run_id": id(1).to_string(),
            "target": { "path": "/projects/api" },
            "discovery": { "runtime": "bun" },
            "created_at": "1970-01-01T00:00:01.000Z",
            "summary": {
                "build_time_ms": 1000,
                "startup_time_ms": 500,
                "memory_peak_bytes": 200,
                "cpu_average_percent": 20.0,
                "requests_per_second": 100.0,
                "p95_ms": 20.0
            }
        })
    );
    assert_eq!(
        data["trends"],
        json!([point(
            id(1),
            [
                json!(1000),
                json!(500),
                json!(200),
                json!(20.0),
                json!(100.0),
                json!(20.0)
            ]
        )])
    );
}

#[tokio::test]
async fn totals_count_the_complete_history_while_trends_hold_the_newest_thirty() {
    let repository = Arc::new(InMemoryRepository::default());
    let ids = seed(&repository, 35);

    let (status, body) = get(&repository, URI).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["totals"]["total_runs"], json!(35));

    let expected: Vec<String> = ids.iter().rev().take(30).map(Uuid::to_string).collect();

    assert_eq!(trend_ids(&body), expected);
    assert_eq!(
        body["data"]["trends"][29]["run_id"],
        json!(id(6).to_string())
    );
}

#[tokio::test]
async fn fewer_than_thirty_runs_are_all_returned_newest_first() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(&repository, 3);

    let (_, body) = get(&repository, URI).await;

    assert_eq!(body["data"]["totals"]["total_runs"], json!(3));
    assert_eq!(
        trend_ids(&body),
        vec![id(3).to_string(), id(2).to_string(), id(1).to_string()]
    );
}

#[tokio::test]
async fn latest_is_the_newest_run_in_the_history_summary_form() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(&repository, 3);

    let (_, overview) = get(&repository, URI).await;
    let (_, history) = get(&repository, "/api/v1/benchmarks?limit=1").await;

    assert_eq!(overview["data"]["latest"], history["data"]["items"][0]);
    assert_eq!(
        overview["data"]["latest"]["run_id"],
        json!(id(3).to_string())
    );
    assert_eq!(
        overview["data"]["trends"][0]["run_id"],
        overview["data"]["latest"]["run_id"]
    );
}

#[tokio::test]
async fn a_failed_build_a_timed_out_startup_and_a_partial_workload_are_null_in_the_trend() {
    let mut run = result(id(1));

    run.metrics.build_time.status = "failed".to_string();
    run.metrics.startup_time.status = "timeout".to_string();
    run.metrics.http_latency.as_mut().unwrap().status = "target_exited".to_string();

    let data = overview_of(run).await;

    // Memory and CPU have no status, so they are kept
    assert_eq!(
        data["trends"],
        json!([point(
            id(1),
            [
                json!(null),
                json!(null),
                json!(200),
                json!(20.0),
                json!(null),
                json!(null)
            ]
        )])
    );

    // The raw history summary behavior is unchanged and still shows the stored values
    assert_eq!(
        data["latest"]["summary"],
        json!({
            "build_time_ms": 1000,
            "startup_time_ms": 500,
            "memory_peak_bytes": 200,
            "cpu_average_percent": 20.0,
            "requests_per_second": 100.0,
            "p95_ms": 20.0
        })
    );
}

#[tokio::test]
async fn only_a_successful_build_has_a_trend_duration() {
    for status in ["failed", "timeout", "no_build_command"] {
        let mut run = result(id(1));

        run.metrics.build_time.status = status.to_string();

        let data = overview_of(run).await;

        assert_eq!(
            data["trends"][0]["build_time_ms"],
            json!(null),
            "build status {status}"
        );
        assert_eq!(data["trends"][0]["startup_time_ms"], json!(500));
    }
}

#[tokio::test]
async fn only_a_successful_startup_has_a_trend_duration() {
    for status in [
        "failed",
        "timeout",
        "unhealthy",
        "no_start_command",
        "no_port",
    ] {
        let mut run = result(id(1));

        run.metrics.startup_time.status = status.to_string();

        let data = overview_of(run).await;

        assert_eq!(
            data["trends"][0]["startup_time_ms"],
            json!(null),
            "startup status {status}"
        );
        assert_eq!(data["trends"][0]["build_time_ms"], json!(1000));
    }
}

#[tokio::test]
async fn zero_observed_requests_are_null_and_not_a_zero_throughput() {
    let mut run = result(id(1));
    let latency = run.metrics.http_latency.as_mut().unwrap();

    latency.total_requests = 0;
    latency.successful_requests = 0;
    latency.failed_requests = 0;
    latency.requests_per_second = 0.0;
    latency.min_ms = None;
    latency.mean_ms = None;
    latency.p50_ms = None;
    latency.p95_ms = None;
    latency.p99_ms = None;
    latency.max_ms = None;

    let data = overview_of(run).await;

    assert_eq!(data["trends"][0]["requests_per_second"], json!(null));
    assert_eq!(data["trends"][0]["p95_ms"], json!(null));
    assert_eq!(data["trends"][0]["memory_peak_bytes"], json!(200));
}

#[tokio::test]
async fn a_run_without_a_workload_has_null_memory_cpu_and_http_metrics() {
    let mut run = result(id(1));

    run.metrics.memory = None;
    run.metrics.cpu = None;
    run.metrics.http_latency = None;

    let data = overview_of(run).await;

    assert_eq!(
        data["trends"],
        json!([point(
            id(1),
            [
                json!(1000),
                json!(500),
                json!(null),
                json!(null),
                json!(null),
                json!(null)
            ]
        )])
    );
}

#[tokio::test]
async fn a_run_where_every_step_was_skipped_has_six_null_metrics() {
    let data = overview_of(skipped(id(1))).await;

    assert_eq!(data["totals"], json!({ "total_runs": 1 }));
    assert_eq!(
        data["trends"],
        json!([point(
            id(1),
            [
                json!(null),
                json!(null),
                json!(null),
                json!(null),
                json!(null),
                json!(null)
            ]
        )])
    );
    assert_eq!(data["latest"]["discovery"], json!({ "runtime": null }));
}

#[tokio::test]
async fn a_measured_zero_stays_zero() {
    let mut run = result(id(1));

    run.metrics.build_time.duration_ms = Some(0);
    run.metrics.memory.as_mut().unwrap().peak.rss_bytes = 0;
    run.metrics.cpu.as_mut().unwrap().average_percent = 0.0;
    run.metrics
        .http_latency
        .as_mut()
        .unwrap()
        .requests_per_second = 0.0;

    let data = overview_of(run).await;
    let trend = &data["trends"][0];

    assert_eq!(trend["build_time_ms"], json!(0));
    assert_eq!(trend["memory_peak_bytes"], json!(0));
    assert_eq!(trend["cpu_average_percent"], json!(0.0));
    assert_eq!(trend["requests_per_second"], json!(0.0));
}

#[tokio::test]
async fn a_storage_failure_is_an_internal_error() {
    let (status, body) = send(Arc::new(FailingRepository), Method::GET, URI).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_error_shape(&body, 500, "INTERNAL_SERVER_ERROR");
}

#[tokio::test]
async fn the_overview_route_rejects_other_methods() {
    let repository = Arc::new(InMemoryRepository::default());

    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let (status, body) = send(repository.clone(), method.clone(), URI).await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "method {method}");
        assert_error_shape(&body, 405, "METHOD_NOT_ALLOWED");
    }
}

#[tokio::test]
async fn query_parameters_are_ignored() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(&repository, 2);

    let (plain_status, plain) = get(&repository, URI).await;
    let (status, body) = get(&repository, &format!("{URI}?limit=1&runtime=bun&unknown=x")).await;

    assert_eq!(plain_status, StatusCode::OK);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"], plain["data"]);
}

#[tokio::test]
async fn the_static_overview_route_does_not_break_the_run_id_route() {
    let repository = Arc::new(InMemoryRepository::default());

    let (status, _) = get(&repository, URI).await;

    assert_eq!(status, StatusCode::OK);

    let (status, body) = get(&repository, "/api/v1/benchmarks/not-a-uuid").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "BAD_REQUEST");

    let uri = format!("/api/v1/benchmarks/{}", Uuid::now_v7());
    let (status, body) = get(&repository, &uri).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error_shape(&body, 404, "NOT_FOUND");
}
