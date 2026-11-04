use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use benchmark_api::{app, config::env::Env, persistence::repository::BenchmarkRepository};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::fake_repository::InMemoryRepository;

fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

fn post(body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/live-runs")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn send(request: Request<Body>) -> (StatusCode, Value) {
    send_with(Arc::new(InMemoryRepository::default()), request).await
}

async fn send_with(
    repository: Arc<dyn BenchmarkRepository>,
    request: Request<Body>,
) -> (StatusCode, Value) {
    let response = app::router(test_env(), repository)
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

fn assert_error_shape(body: &Value, http_code: u64, code: &str) {
    assert_eq!(body["success"], json!(false));
    assert_eq!(body["httpCode"], json!(http_code));
    assert_eq!(body["code"], json!(code));
    assert!(body["message"].is_string());
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn wrong_field_type_is_a_bad_request_with_the_serde_detail() {
    let body = r#"{
        "path": "/tmp",
        "startup": {
            "command": { "program": "bun", "args": [] },
            "readiness": {
                "type": "http",
                "host": "127.0.0.1",
                "port": "8090",
                "path": "/",
                "method": "GET",
                "expected_status": 200
            }
        }
    }"#;

    let (status, body) = send(post(body)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "BAD_REQUEST");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("startup.readiness.port")
    );
}

#[tokio::test]
async fn missing_required_field_is_a_bad_request() {
    let (status, body) = send(post("{}")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "BAD_REQUEST");
}

#[tokio::test]
async fn malformed_json_is_a_bad_request() {
    let (status, body) = send(post("{ not json")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "BAD_REQUEST");
}

#[tokio::test]
async fn missing_content_type_is_a_bad_request() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/live-runs")
        .body(Body::from(r#"{"path":"/tmp"}"#))
        .unwrap();

    let (status, body) = send(request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "BAD_REQUEST");
}

#[tokio::test]
async fn invalid_workload_is_a_validation_error() {
    let body = json!({
        "path": "/tmp",
        "workload": {
            "method": "GET",
            "path": "/health",
            "requests": 0,
            "concurrency": 1
        }
    })
    .to_string();

    let (status, body) = send(post(&body)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
    assert!(body["error"].as_str().unwrap().contains("requests"));
}

#[tokio::test]
async fn missing_target_path_is_a_bad_request() {
    let body = json!({ "path": "/definitely/not/a/real/path" }).to_string();

    let (status, body) = send(post(&body)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "BAD_REQUEST");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("Path does not exist")
    );
}

#[tokio::test]
async fn unknown_route_is_a_not_found_error() {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/unknown")
        .body(Body::empty())
        .unwrap();

    let (status, body) = send(request).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error_shape(&body, 404, "NOT_FOUND");
}

#[tokio::test]
async fn wrong_method_is_a_method_not_allowed_error() {
    let request = Request::builder()
        .method(Method::DELETE)
        .uri("/api/v1/benchmarks")
        .body(Body::empty())
        .unwrap();

    let (status, body) = send(request).await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_error_shape(&body, 405, "METHOD_NOT_ALLOWED");
}

#[tokio::test]
async fn requests_together_with_duration_is_a_validation_error() {
    let body = json!({
        "path": "/tmp",
        "workload": {
            "method": "GET",
            "path": "/health",
            "requests": 10,
            "duration_seconds": 5,
            "concurrency": 2
        }
    })
    .to_string();

    let (status, body) = send(post(&body)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("mutually exclusive")
    );
}

#[tokio::test]
async fn duration_above_the_maximum_is_a_validation_error() {
    let body = json!({
        "path": "/tmp",
        "workload": {
            "method": "GET",
            "path": "/health",
            "duration_seconds": 301,
            "concurrency": 2
        }
    })
    .to_string();

    let (status, body) = send(post(&body)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
}

#[tokio::test]
async fn zero_duration_is_a_validation_error() {
    let body = json!({
        "path": "/tmp",
        "workload": {
            "method": "GET",
            "path": "/health",
            "duration_seconds": 0,
            "concurrency": 2
        }
    })
    .to_string();

    let (status, body) = send(post(&body)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
}

#[tokio::test]
async fn workload_without_requests_or_duration_is_a_validation_error() {
    let body = json!({
        "path": "/tmp",
        "workload": { "method": "GET", "path": "/health", "concurrency": 2 }
    })
    .to_string();

    let (status, body) = send(post(&body)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
}

#[tokio::test]
async fn rejected_requests_are_not_persisted() {
    let repository = Arc::new(InMemoryRepository::default());

    let (status, _) = send_with(repository.clone(), post("{}")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test]
async fn the_legacy_blocking_post_is_gone() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/benchmarks")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"path":"/tmp"}"#))
        .unwrap();

    let (status, body) = send(request).await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_error_shape(&body, 405, "METHOD_NOT_ALLOWED");
}
