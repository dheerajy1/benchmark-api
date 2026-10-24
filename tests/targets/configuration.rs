use axum::http::StatusCode;
use tempfile::tempdir;

use super::support::{get_path, policy, router, send};

#[tokio::test]
async fn an_unknown_run_has_no_configuration_and_is_not_found() {
    let dir = tempdir().unwrap();
    let (app, _) = router(policy(false, dir.path()));

    let path = format!("/api/v1/benchmarks/{}/config", uuid::Uuid::now_v7());
    let (status, value) = send(app, get_path(&path)).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(value["code"], "NOT_FOUND");
}

#[tokio::test]
async fn a_malformed_run_id_is_a_bad_request() {
    let dir = tempdir().unwrap();
    let (app, _) = router(policy(false, dir.path()));

    let (status, value) = send(app, get_path("/api/v1/benchmarks/not-a-uuid/config")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(value["code"], "BAD_REQUEST");
}

#[tokio::test]
async fn the_listing_is_empty_when_no_run_exists() {
    let dir = tempdir().unwrap();
    let (app, _) = router(policy(false, dir.path()));

    let (status, value) = send(app, get_path("/api/v1/benchmarks/configurations")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"], serde_json::json!([]));
}

#[tokio::test]
async fn the_listing_limit_must_be_between_one_and_two_hundred() {
    for path in [
        "/api/v1/benchmarks/configurations?limit=0",
        "/api/v1/benchmarks/configurations?limit=201",
        "/api/v1/benchmarks/configurations?limit=abc",
    ] {
        let dir = tempdir().unwrap();
        let (app, _) = router(policy(false, dir.path()));

        let (status, value) = send(app, get_path(path)).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(value["code"], "VALIDATION_ERROR", "{path}");
    }
}
