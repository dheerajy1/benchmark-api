// The configuration endpoints through the real router and the real repository, on the
// isolated test schema. Nothing here uses the in-memory fakes.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use benchmark_api::{
    app,
    config::env::Env,
    persistence::{postgres::repository::PgBenchmarkRepository, repository::BenchmarkRepository},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{
    sample,
    support::{TestDatabase, setup},
};

fn env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

// The full application router over the test database
fn router(database: &TestDatabase) -> Router {
    let repository: Arc<dyn BenchmarkRepository> =
        Arc::new(PgBenchmarkRepository::new(database.pool.clone()));

    app::router(env(), repository)
}

async fn get(router: &Router, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn a_saved_target_configuration_is_returned_by_the_detail_endpoint() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();
    let configuration = sample::target_configuration();

    repository.save(&result, &configuration).await.unwrap();

    let router = router(&database);
    let path = format!("/api/v1/benchmarks/{}/config", result.run_id);
    let (status, body) = get(&router, &path).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["run_id"], result.run_id.to_string());
    assert_eq!(body["data"]["recorded"], true);
    assert_eq!(
        body["data"]["configuration"],
        serde_json::to_value(&configuration).unwrap()
    );

    database.teardown().await;
}

#[tokio::test]
async fn the_listing_returns_saved_configurations_newest_first() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let older = sample::result();
    repository
        .save(&older, &sample::configuration())
        .await
        .unwrap();

    // Run ids are UUIDv7 and carry millisecond timestamps, so the two runs must not share one
    tokio::time::sleep(Duration::from_millis(5)).await;

    let newer = sample::result();
    repository
        .save(&newer, &sample::target_configuration())
        .await
        .unwrap();

    let router = router(&database);
    let (status, body) = get(&router, "/api/v1/benchmarks/configurations?limit=10").await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let items = body["data"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{body}");

    assert_eq!(items[0]["run_id"], newer.run_id.to_string());
    assert_eq!(items[1]["run_id"], older.run_id.to_string());

    assert_eq!(items[0]["recorded"], true);
    assert_eq!(
        items[0]["configuration"],
        serde_json::to_value(sample::target_configuration()).unwrap()
    );
    assert_eq!(
        items[1]["configuration"],
        serde_json::to_value(sample::configuration()).unwrap()
    );

    // created_at is an RFC 3339 timestamp
    assert!(items[0]["created_at"].as_str().unwrap().contains('T'));

    let (status, limited) = get(&router, "/api/v1/benchmarks/configurations?limit=1").await;

    assert_eq!(status, StatusCode::OK, "{limited}");
    assert_eq!(limited["data"].as_array().unwrap().len(), 1);
    assert_eq!(limited["data"][0]["run_id"], newer.run_id.to_string());

    database.teardown().await;
}

#[tokio::test]
async fn a_run_stored_without_a_configuration_is_not_recorded_over_http() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository
        .save(&result, &sample::configuration())
        .await
        .unwrap();

    // Simulates a run stored before configurations were recorded
    sqlx::query("DELETE FROM benchmark_run_configurations WHERE run_id = $1")
        .bind(result.run_id)
        .execute(&database.pool)
        .await
        .unwrap();

    let router = router(&database);

    let path = format!("/api/v1/benchmarks/{}/config", result.run_id);
    let (status, body) = get(&router, &path).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["recorded"], false);
    assert_eq!(body["data"]["configuration"], json!(null));

    let (status, listed) = get(&router, "/api/v1/benchmarks/configurations?limit=10").await;

    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["data"][0]["run_id"], result.run_id.to_string());
    assert_eq!(listed["data"][0]["recorded"], false);
    assert_eq!(listed["data"][0]["configuration"], json!(null));

    database.teardown().await;
}

#[tokio::test]
async fn an_unknown_run_is_not_found_and_a_malformed_id_is_a_bad_request_over_http() {
    let Some(database) = setup().await else {
        return;
    };

    let router = router(&database);

    let unknown = format!("/api/v1/benchmarks/{}/config", uuid::Uuid::now_v7());
    let (status, body) = get(&router, &unknown).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "NOT_FOUND");

    let (status, body) = get(&router, "/api/v1/benchmarks/not-a-uuid/config").await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "BAD_REQUEST");

    database.teardown().await;
}
