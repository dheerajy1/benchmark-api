use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use benchmark_api::{app, config::env::Env, persistence::repository::BenchmarkRepository};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::fake_repository::{FailingRepository, InMemoryRepository};

const URI: &str = "/api/v1/health";

async fn send(repository: Arc<dyn BenchmarkRepository>, method: Method) -> (StatusCode, Value) {
    let env = Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    };
    let request = Request::builder()
        .method(method)
        .uri(URI)
        .body(Body::empty())
        .unwrap();

    let response = app::router(env, repository).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn a_reachable_database_is_healthy_and_the_version_is_the_crate_version() {
    let (status, body) = send(Arc::new(InMemoryRepository::default()), Method::GET).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["httpCode"], json!(200));
    assert_eq!(
        body["data"],
        json!({ "ok": true, "database": true, "version": env!("CARGO_PKG_VERSION") })
    );
}

#[tokio::test]
async fn an_unreachable_database_is_a_503_in_the_error_shape() {
    let (status, body) = send(Arc::new(FailingRepository), Method::GET).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["success"], json!(false));
    assert_eq!(body["httpCode"], json!(503));
    assert_eq!(body["code"], json!("SERVICE_UNAVAILABLE"));
    assert!(body["message"].is_string());
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn other_methods_are_not_allowed() {
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let (status, body) = send(Arc::new(InMemoryRepository::default()), method).await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["code"], json!("METHOD_NOT_ALLOWED"));
    }
}
