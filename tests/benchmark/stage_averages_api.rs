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
    comparison_sample::result,
    fake_repository::{FailingRepository, InMemoryRepository},
};

const URI: &str = "/api/v1/benchmarks/stage-averages";
const PATH_URI: &str = "/api/v1/benchmarks/stage-averages?target_path=/projects/api";

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

// The id of run number `number`, one second after the previous number
fn id(number: u64) -> Uuid {
    let value =
        (u128::from(number * 1000) << 80) | (0x7u128 << 76) | (0b10u128 << 62) | u128::from(number);

    Uuid::from_u128(value)
}

// Stores a done run with the given stage durations
fn seed(repository: &InMemoryRepository, number: u64, stages: &[(&str, Option<u64>)]) -> Uuid {
    repository.seed_result(result(id(number)));
    repository.seed_stages(id(number), stages);

    id(number)
}

fn failed(run_id: Uuid) -> BenchmarkResult {
    let mut run = result(run_id);

    run.metrics.build_time.as_mut().unwrap().status = "failed".to_string();

    run
}

#[tokio::test]
async fn a_target_without_done_runs_is_unknown() {
    let repository = Arc::new(InMemoryRepository::default());

    let (status, body) = get(&repository, PATH_URI).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["message"], json!("Stage averages retrieved."));
    assert_eq!(
        body["data"],
        json!({ "runs_used": 0, "stages": [], "total_average_ms": null })
    );
}

#[tokio::test]
async fn fewer_than_three_runs_average_what_exists() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(
        &repository,
        1,
        &[("build", Some(1000)), ("startup", Some(500))],
    );
    seed(
        &repository,
        2,
        &[("build", Some(2000)), ("startup", Some(700))],
    );

    let (_, body) = get(&repository, PATH_URI).await;

    assert_eq!(
        body["data"],
        json!({
            "runs_used": 2,
            "stages": [
                { "stage": "build", "average_ms": 1500, "last_ms": 2000 },
                { "stage": "startup", "average_ms": 600, "last_ms": 700 }
            ],
            "total_average_ms": 2100
        })
    );
}

#[tokio::test]
async fn only_the_newest_three_runs_are_used() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(&repository, 1, &[("build", Some(100_000))]);
    seed(&repository, 2, &[("build", Some(1000))]);
    seed(&repository, 3, &[("build", Some(2000))]);
    seed(&repository, 4, &[("build", Some(3000))]);

    let (_, body) = get(&repository, PATH_URI).await;

    assert_eq!(body["data"]["runs_used"], json!(3));
    assert_eq!(
        body["data"]["stages"],
        json!([{ "stage": "build", "average_ms": 2000, "last_ms": 3000 }])
    );
    assert_eq!(body["data"]["total_average_ms"], json!(2000));
}

#[tokio::test]
async fn a_deleted_run_is_excluded() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(&repository, 1, &[("build", Some(1000))]);
    let newest = seed(&repository, 2, &[("build", Some(9000))]);

    let (status, _) = send(
        repository.clone(),
        Method::DELETE,
        &format!("/api/v1/benchmarks/{newest}"),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let (_, body) = get(&repository, PATH_URI).await;

    assert_eq!(body["data"]["runs_used"], json!(1));
    assert_eq!(
        body["data"]["stages"],
        json!([{ "stage": "build", "average_ms": 1000, "last_ms": 1000 }])
    );
}

#[tokio::test]
async fn a_failed_run_is_excluded() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(&repository, 1, &[("build", Some(1000))]);
    repository.seed_result(failed(id(2)));
    repository.seed_stages(id(2), &[("build", Some(9000))]);

    let (_, body) = get(&repository, PATH_URI).await;

    assert_eq!(body["data"]["runs_used"], json!(1));
    assert_eq!(
        body["data"]["stages"],
        json!([{ "stage": "build", "average_ms": 1000, "last_ms": 1000 }])
    );
}

#[tokio::test]
async fn a_stage_missing_from_some_runs_is_averaged_over_the_runs_that_have_it() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(
        &repository,
        1,
        &[("build", Some(1000)), ("startup", Some(500))],
    );
    seed(&repository, 2, &[("build", Some(3000))]);

    let (_, body) = get(&repository, PATH_URI).await;

    assert_eq!(body["data"]["runs_used"], json!(2));
    // The newest run has no startup, so its last_ms is null and the average uses the one run
    assert_eq!(
        body["data"]["stages"],
        json!([
            { "stage": "build", "average_ms": 2000, "last_ms": 3000 },
            { "stage": "startup", "average_ms": 500, "last_ms": null }
        ])
    );
    // The totals of the runs are 1500 and 3000
    assert_eq!(body["data"]["total_average_ms"], json!(2250));
}

#[tokio::test]
async fn a_stage_without_a_duration_is_skipped_and_never_a_zero() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(&repository, 1, &[("build", Some(1000)), ("startup", None)]);
    seed(&repository, 2, &[("build", Some(2000)), ("startup", None)]);

    let (_, body) = get(&repository, PATH_URI).await;

    assert_eq!(
        body["data"]["stages"],
        json!([{ "stage": "build", "average_ms": 1500, "last_ms": 2000 }])
    );
}

#[tokio::test]
async fn a_done_run_without_stage_rows_counts_as_a_run_but_has_no_total() {
    let repository = Arc::new(InMemoryRepository::default());

    repository.seed_result(result(id(1)));

    let (_, body) = get(&repository, PATH_URI).await;

    assert_eq!(
        body["data"],
        json!({ "runs_used": 1, "stages": [], "total_average_ms": null })
    );
}

#[tokio::test]
async fn averages_are_rounded_to_the_nearest_millisecond_half_up() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(&repository, 1, &[("build", Some(1))]);
    seed(&repository, 2, &[("build", Some(2))]);

    let (_, body) = get(&repository, PATH_URI).await;

    assert_eq!(body["data"]["stages"][0]["average_ms"], json!(2));
}

#[tokio::test]
async fn another_target_is_never_mixed_in_and_a_trailing_slash_matches() {
    let repository = Arc::new(InMemoryRepository::default());

    seed(&repository, 1, &[("build", Some(1000))]);

    let mut other = result(id(2));

    other.target.path = "/projects/web".to_string();
    repository.seed_result(other);
    repository.seed_stages(id(2), &[("build", Some(9000))]);

    let (_, body) = get(&repository, &format!("{URI}?target_path=/projects/api/")).await;

    assert_eq!(body["data"]["runs_used"], json!(1));
    assert_eq!(body["data"]["stages"][0]["average_ms"], json!(1000));
}

#[tokio::test]
async fn the_target_path_is_required_and_not_empty() {
    let repository = Arc::new(InMemoryRepository::default());

    for uri in [URI.to_string(), format!("{URI}?target_path=")] {
        let (status, body) = get(&repository, &uri).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_error_shape(&body, 400, "VALIDATION_ERROR");
    }
}

#[tokio::test]
async fn a_storage_failure_is_an_internal_error() {
    let (status, body) = send(Arc::new(FailingRepository), Method::GET, PATH_URI).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_error_shape(&body, 500, "INTERNAL_SERVER_ERROR");
}

#[tokio::test]
async fn other_methods_are_not_allowed_and_the_run_id_route_is_not_shadowed() {
    let repository = Arc::new(InMemoryRepository::default());

    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let (status, body) = send(repository.clone(), method.clone(), PATH_URI).await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "method {method}");
        assert_error_shape(&body, 405, "METHOD_NOT_ALLOWED");
    }

    let (status, body) = get(&repository, "/api/v1/benchmarks/not-a-uuid").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "BAD_REQUEST");
}
