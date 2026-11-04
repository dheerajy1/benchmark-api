use std::{collections::HashSet, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode},
};
use benchmark_api::{app, config::env::Env, origin, persistence::repository::BenchmarkRepository};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::fake_repository::{FailingRepository, InMemoryRepository};

fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

// Sends one request with extra headers, returns status, headers and the JSON body (Null when
// the body is empty)
async fn send(
    repository: Arc<dyn BenchmarkRepository>,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = Request::builder().method(method).uri(uri);

    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }

    let request = builder.body(Body::from(body.to_string())).unwrap();
    let response = app::router(test_env(), repository)
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    (status, headers, body)
}

// A repository, a method, a uri, a body and the status the request must answer with
type Case<'a> = (
    Arc<dyn BenchmarkRepository>,
    Method,
    &'a str,
    &'a str,
    StatusCode,
);

fn memory() -> Arc<dyn BenchmarkRepository> {
    Arc::new(InMemoryRepository::default())
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .expect("every answer carries x-request-id")
        .to_str()
        .unwrap()
        .to_string()
}

fn is_short_id(id: &str) -> bool {
    id.len() == 8
        && id
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
}

#[tokio::test]
async fn a_successful_answer_has_a_short_id_header_and_no_id_in_the_body() {
    let (status, headers, body) = send(memory(), Method::GET, "/api/v1/health", &[], "").await;

    assert_eq!(status, StatusCode::OK);
    assert!(is_short_id(&request_id(&headers)));
    assert!(body.get("requestId").is_none());
}

#[tokio::test]
async fn every_error_carries_the_same_id_in_the_header_and_the_body() {
    let cases: Vec<Case> = vec![
        (
            memory(),
            Method::GET,
            "/api/v1/nowhere",
            "",
            StatusCode::NOT_FOUND,
        ),
        (
            memory(),
            Method::PUT,
            "/api/v1/benchmarks",
            "",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            memory(),
            Method::GET,
            "/api/v1/benchmarks?status=nope",
            "",
            StatusCode::BAD_REQUEST,
        ),
        (
            memory(),
            Method::POST,
            "/api/v1/live-runs",
            "{ not json",
            StatusCode::BAD_REQUEST,
        ),
        (
            Arc::new(FailingRepository) as Arc<dyn BenchmarkRepository>,
            Method::GET,
            "/api/v1/benchmarks",
            "",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            Arc::new(FailingRepository) as Arc<dyn BenchmarkRepository>,
            Method::GET,
            "/api/v1/health",
            "",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    ];

    for (repository, method, uri, body, expected) in cases {
        let headers = if body.is_empty() {
            vec![]
        } else {
            vec![("content-type", "application/json")]
        };
        let (status, headers, body) = send(repository, method, uri, &headers, body).await;

        assert_eq!(status, expected, "{uri}");
        assert_eq!(body["success"], json!(false), "{uri}");

        let id = request_id(&headers);

        assert!(is_short_id(&id), "{uri}");
        assert_eq!(body["requestId"], json!(id), "{uri}");
    }
}

#[tokio::test]
async fn every_request_gets_its_own_id() {
    let mut ids = HashSet::new();

    for _ in 0..50 {
        let (_, headers, _) = send(memory(), Method::GET, "/api/v1/health", &[], "").await;

        ids.insert(request_id(&headers));
    }

    assert_eq!(ids.len(), 50);
}

#[tokio::test]
async fn an_id_sent_by_the_client_is_ignored() {
    let (_, headers, body) = send(
        memory(),
        Method::GET,
        "/api/v1/nowhere",
        &[("x-request-id", "client-chosen")],
        "",
    )
    .await;

    let id = request_id(&headers);

    assert_ne!(id, "client-chosen");
    assert_eq!(body["requestId"], json!(id));
}

#[tokio::test]
async fn the_dashboard_origins_may_use_every_method_the_dashboard_needs() {
    let run = Uuid::now_v7();
    let uri = format!("/api/v1/benchmarks/{run}");

    for allowed in [
        "http://localhost:5003",
        "http://127.0.0.1:5003",
        "http://localhost:8093",
        "http://127.0.0.1:8093",
        "http://localhost:5002",
    ] {
        let (status, headers, _) = send(
            memory(),
            Method::OPTIONS,
            &uri,
            &[
                ("origin", allowed),
                ("access-control-request-method", "DELETE"),
            ],
            "",
        )
        .await;

        assert!(status.is_success(), "{allowed} preflight, got {status}");
        assert_eq!(headers.get("access-control-allow-origin").unwrap(), allowed);

        let methods = headers
            .get("access-control-allow-methods")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        for method in ["GET", "POST", "DELETE"] {
            assert!(methods.contains(method), "{allowed} must allow {method}");
        }
    }
}

#[tokio::test]
async fn the_request_id_header_is_exposed_to_the_browser() {
    let (_, headers, _) = send(
        memory(),
        Method::GET,
        "/api/v1/health",
        &[("origin", "http://localhost:5003")],
        "",
    )
    .await;

    assert_eq!(
        headers.get("access-control-allow-origin").unwrap(),
        "http://localhost:5003"
    );

    let exposed = headers
        .get("access-control-expose-headers")
        .expect("the id header must be exposed")
        .to_str()
        .unwrap()
        .to_ascii_lowercase();

    assert!(exposed.contains("x-request-id"));
}

#[tokio::test]
async fn an_origin_outside_the_list_gets_no_cors_headers() {
    for refused in ["http://localhost:5004", "https://evil.example.com"] {
        let (_, headers, _) = send(
            memory(),
            Method::GET,
            "/api/v1/health",
            &[("origin", refused)],
            "",
        )
        .await;

        assert!(
            headers.get("access-control-allow-origin").is_none(),
            "{refused}"
        );
    }
}

#[test]
fn the_shared_allow_list_holds_the_dashboard_origins_and_nothing_near_them() {
    for allowed in [
        "http://localhost:5003",
        "http://127.0.0.1:5003",
        "http://localhost:8093",
        "http://127.0.0.1:8093",
    ] {
        assert!(origin::is_allowed(allowed), "{allowed}");
    }

    for refused in [
        "http://localhost:5004",
        "https://localhost:5003",
        "http://localhost:50030",
        "http://evil.localhost:5003",
    ] {
        assert!(!origin::is_allowed(refused), "{refused}");
    }
}
