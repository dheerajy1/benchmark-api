use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use benchmark_api::{app, config::env::Env, persistence::repository::BenchmarkRepository};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{
    comparison_sample::{result, skipped},
    fake_repository::{FailingRepository, InMemoryRepository},
};

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

fn uri(ids: &str) -> String {
    format!("/api/v1/comparisons/summary?runIds={ids}")
}

// Two stored runs, the comparison run builds 50 percent slower
fn seeded() -> (Arc<InMemoryRepository>, Uuid, Uuid) {
    let repository = Arc::new(InMemoryRepository::default());

    let baseline = Uuid::now_v7();
    let comparison = Uuid::now_v7();

    repository.seed_result(result(baseline));

    let mut slower = result(comparison);

    slower.metrics.build_time.duration_ms = Some(1500);

    repository.seed_result(slower);

    (repository, baseline, comparison)
}

#[tokio::test]
async fn two_runs_are_compared() {
    let (repository, baseline, comparison) = seeded();

    let (status, body) = get(&repository, &uri(&format!("{baseline},{comparison}"))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["httpCode"], json!(200));
    assert_eq!(body["message"], json!("Comparison completed."));

    let data = &body["data"];

    assert_eq!(data["baseline"]["run_id"], json!(baseline.to_string()));
    assert_eq!(data["comparison"]["run_id"], json!(comparison.to_string()));
    assert_eq!(
        data["baseline"]["target"],
        json!({ "path": "/projects/api" })
    );
    assert_eq!(data["baseline"]["runtime"], json!("bun"));
    assert!(data["baseline"]["created_at"].is_string());

    let build = &data["metrics"]["build_time"]["duration_ms"];

    assert_eq!(build["baseline_value"], json!(1000));
    assert_eq!(build["comparison_value"], json!(1500));
    assert_eq!(build["absolute_difference"], json!(500));
    assert_eq!(build["percentage_change"], json!(50.0));
    assert_eq!(build["direction"], json!("lower_is_better"));

    let groups: Vec<&String> = data["metrics"].as_object().unwrap().keys().collect();

    for group in [
        "build_time",
        "startup_time",
        "memory",
        "cpu",
        "http_latency",
        "artifact_size",
    ] {
        assert!(groups.iter().any(|key| key.as_str() == group), "{group}");
    }
}

#[tokio::test]
async fn the_order_of_the_ids_decides_baseline_and_comparison() {
    let (repository, first, second) = seeded();

    let (_, forward) = get(&repository, &uri(&format!("{first},{second}"))).await;
    let (_, backward) = get(&repository, &uri(&format!("{second},{first}"))).await;

    assert_eq!(
        forward["data"]["baseline"]["run_id"],
        json!(first.to_string())
    );
    assert_eq!(
        backward["data"]["baseline"]["run_id"],
        json!(second.to_string())
    );

    assert_eq!(
        forward["data"]["metrics"]["build_time"]["duration_ms"]["absolute_difference"],
        json!(500)
    );
    assert_eq!(
        backward["data"]["metrics"]["build_time"]["duration_ms"]["absolute_difference"],
        json!(-500)
    );
}

#[tokio::test]
async fn a_comparison_is_never_stored() {
    let (repository, baseline, comparison) = seeded();

    get(&repository, &uri(&format!("{baseline},{comparison}"))).await;

    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test]
async fn no_run_ids_are_rejected() {
    let (repository, _, _) = seeded();

    for uri in [
        "/api/v1/comparisons/summary".to_string(),
        "/api/v1/comparisons/summary?runIds=".to_string(),
    ] {
        let (status, body) = get(&repository, &uri).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_error_shape(&body, 400, "VALIDATION_ERROR");
    }
}

#[tokio::test]
async fn one_run_id_is_rejected() {
    let (repository, baseline, _) = seeded();

    let (status, body) = get(&repository, &uri(&baseline.to_string())).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
}

#[tokio::test]
async fn three_run_ids_are_rejected() {
    let (repository, baseline, comparison) = seeded();

    let ids = format!("{baseline},{comparison},{}", Uuid::now_v7());

    let (status, body) = get(&repository, &uri(&ids)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
}

#[tokio::test]
async fn duplicate_run_ids_are_rejected_even_with_different_letter_case() {
    let (repository, baseline, _) = seeded();

    let lower = baseline.to_string();
    let upper = lower.to_uppercase();

    for ids in [format!("{lower},{lower}"), format!("{lower},{upper}")] {
        let (status, body) = get(&repository, &uri(&ids)).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{ids}");
        assert_error_shape(&body, 400, "VALIDATION_ERROR");
    }
}

#[tokio::test]
async fn malformed_ids_are_rejected_without_normalization() {
    let (repository, baseline, comparison) = seeded();

    let cases = [
        format!("{baseline},not-a-uuid"),
        format!("not-a-uuid,{comparison}"),
        // trailing comma leaves an empty segment
        format!("{baseline},"),
        // a space is not trimmed
        format!("{baseline},%20{comparison}"),
    ];

    for ids in cases {
        let (status, body) = get(&repository, &uri(&ids)).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{ids}");
        assert_error_shape(&body, 400, "VALIDATION_ERROR");
    }
}

#[tokio::test]
async fn a_repeated_run_ids_parameter_is_rejected() {
    let (repository, baseline, comparison) = seeded();

    let uri = format!("/api/v1/comparisons/summary?runIds={baseline}&runIds={comparison}");

    let (status, body) = get(&repository, &uri).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
}

#[tokio::test]
async fn a_missing_baseline_run_is_not_found() {
    let (repository, _, comparison) = seeded();
    let missing = Uuid::now_v7();

    let (status, body) = get(&repository, &uri(&format!("{missing},{comparison}"))).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error_shape(&body, 404, "NOT_FOUND");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains(&missing.to_string())
    );
}

#[tokio::test]
async fn a_missing_comparison_run_is_not_found() {
    let (repository, baseline, _) = seeded();
    let missing = Uuid::now_v7();

    let (status, body) = get(&repository, &uri(&format!("{baseline},{missing}"))).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error_shape(&body, 404, "NOT_FOUND");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains(&missing.to_string())
    );
}

#[tokio::test]
async fn when_both_runs_are_missing_both_ids_are_named() {
    let repository = Arc::new(InMemoryRepository::default());
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();

    let (status, body) = get(&repository, &uri(&format!("{first},{second}"))).await;

    assert_eq!(status, StatusCode::NOT_FOUND);

    let error = body["error"].as_str().unwrap();

    assert!(error.contains(&first.to_string()));
    assert!(error.contains(&second.to_string()));
}

#[tokio::test]
async fn a_missing_metric_is_null_through_the_api() {
    let repository = Arc::new(InMemoryRepository::default());
    let baseline = Uuid::now_v7();
    let comparison = Uuid::now_v7();

    repository.seed_result(result(baseline));

    let mut without_memory = result(comparison);

    without_memory.metrics.memory = None;

    repository.seed_result(without_memory);

    let (status, body) = get(&repository, &uri(&format!("{baseline},{comparison}"))).await;

    assert_eq!(status, StatusCode::OK);

    let peak = &body["data"]["metrics"]["memory"]["peak_rss_bytes"];

    assert_eq!(peak["baseline_value"], json!(200));
    assert_eq!(peak["comparison_value"], json!(null));
    assert_eq!(peak["absolute_difference"], json!(null));
    assert_eq!(peak["percentage_change"], json!(null));
}

#[tokio::test]
async fn a_skipped_run_does_not_fail_the_comparison() {
    let repository = Arc::new(InMemoryRepository::default());
    let baseline = Uuid::now_v7();
    let comparison = Uuid::now_v7();

    repository.seed_result(result(baseline));
    repository.seed_result(skipped(comparison));

    let (status, body) = get(&repository, &uri(&format!("{baseline},{comparison}"))).await;

    assert_eq!(status, StatusCode::OK);

    let metrics = &body["data"]["metrics"];

    assert_eq!(
        metrics["build_time"]["comparison_status"],
        json!("no_build_command")
    );
    assert_eq!(
        metrics["build_time"]["duration_ms"]["comparison_value"],
        json!(null)
    );
    assert_eq!(
        metrics["startup_time"]["comparison_status"],
        json!("no_start_command")
    );
    assert_eq!(body["data"]["comparison"]["runtime"], json!(null));
}

#[tokio::test]
async fn a_storage_failure_is_an_internal_error() {
    let uri = uri(&format!("{},{}", Uuid::now_v7(), Uuid::now_v7()));

    let (status, body) = send(Arc::new(FailingRepository), Method::GET, &uri).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_error_shape(&body, 500, "INTERNAL_SERVER_ERROR");
}

#[tokio::test]
async fn other_methods_are_not_allowed() {
    let repository = Arc::new(InMemoryRepository::default());
    let uri = uri(&format!("{},{}", Uuid::now_v7(), Uuid::now_v7()));

    for method in [Method::POST, Method::PUT, Method::DELETE] {
        let (status, body) = send(repository.clone(), method, &uri).await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_error_shape(&body, 405, "METHOD_NOT_ALLOWED");
    }
}
