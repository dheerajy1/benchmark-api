use axum::http::StatusCode;
use serde_json::{Value, json};
use tempfile::tempdir;

use super::support::{get_path, policy, post_json, router, send, wait_for_seen};

#[tokio::test]
async fn a_raw_request_is_refused_when_custom_runs_are_disabled() {
    let dir = tempdir().unwrap();
    let (app, executor) = router(policy(false, dir.path()));
    let body = json!({ "path": dir.path().display().to_string() }).to_string();

    let (status, value) = send(app, post_json("/api/v1/live-runs", &body)).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(value["code"], "FORBIDDEN");
    assert_eq!(
        value["error"],
        "Custom runs are disabled on this server, use a saved target."
    );
    assert!(executor.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_raw_request_is_accepted_and_recorded_as_custom_when_enabled() {
    let dir = tempdir().unwrap();
    let (app, executor) = router(policy(true, dir.path()));
    let body = json!({ "path": dir.path().display().to_string() }).to_string();

    let (status, accepted) = send(app.clone(), post_json("/api/v1/live-runs", &body)).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let run_id = accepted["data"]["run_id"].as_str().unwrap().to_string();
    let (_, replay) = send(app, get_path(&format!("/api/v1/live-runs/{run_id}"))).await;

    let accepted_event = &replay["data"]["events"][0];
    assert_eq!(accepted_event["type"], "run.accepted");
    assert_eq!(accepted_event["data"]["source"], "custom");
    assert!(accepted_event["data"]["target"]["id"].is_null());

    wait_for_seen(&executor, 1).await;
}

#[tokio::test]
async fn a_saved_target_runs_while_custom_runs_are_disabled() {
    let dir = tempdir().unwrap();
    let (app, executor) = router(policy(false, dir.path()));

    let (status, accepted) = send(
        app.clone(),
        post_json("/api/v1/live-runs", r#"{"target_id":"demo"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let run_id = accepted["data"]["run_id"].as_str().unwrap().to_string();
    let (_, replay) = send(app, get_path(&format!("/api/v1/live-runs/{run_id}"))).await;
    let accepted_event = &replay["data"]["events"][0];

    assert_eq!(accepted_event["data"]["source"], "target");
    assert_eq!(accepted_event["data"]["target"]["id"], "demo");
    assert_eq!(accepted_event["data"]["target"]["name"], "Demo API");

    wait_for_seen(&executor, 1).await;
    let seen = executor.seen.lock().unwrap().clone();
    let workload = seen[0].workload.as_ref().unwrap();

    assert_eq!(workload.duration_seconds, Some(30));
    assert_eq!(workload.concurrency, 50);
}

#[tokio::test]
async fn a_request_override_replaces_the_duration_and_keeps_the_target_settings() {
    let dir = tempdir().unwrap();
    let (app, executor) = router(policy(false, dir.path()));

    let body = r#"{"target_id":"demo","workload":{"requests":10}}"#;
    let (status, _) = send(app, post_json("/api/v1/live-runs", body)).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    wait_for_seen(&executor, 1).await;
    let seen = executor.seen.lock().unwrap().clone();
    let workload = seen[0].workload.as_ref().unwrap();

    assert_eq!(workload.requests, Some(10));
    assert_eq!(workload.duration_seconds, None);
    assert_eq!(workload.concurrency, 50);
    assert_eq!(workload.path, "/health");
    assert_eq!(workload.method, "GET");
}

#[tokio::test]
async fn a_concurrency_override_keeps_the_target_duration() {
    let dir = tempdir().unwrap();
    let (app, executor) = router(policy(false, dir.path()));

    let body = r#"{"target_id":"demo","workload":{"concurrency":7}}"#;
    let (status, _) = send(app, post_json("/api/v1/live-runs", body)).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    wait_for_seen(&executor, 1).await;
    let seen = executor.seen.lock().unwrap().clone();
    let workload = seen[0].workload.as_ref().unwrap();

    assert_eq!(workload.duration_seconds, Some(30));
    assert_eq!(workload.concurrency, 7);
}

#[tokio::test]
async fn a_target_body_cannot_override_the_path_startup_or_headers() {
    let dir = tempdir().unwrap();
    let cases = [
        r#"{"target_id":"demo","path":"/tmp"}"#,
        r#"{"target_id":"demo","startup":{}}"#,
        r#"{"target_id":"demo","workload":{"headers":{"X-A":"b"}}}"#,
    ];

    for body in cases {
        let (app, executor) = router(policy(false, dir.path()));

        let (status, value) = send(app, post_json("/api/v1/live-runs", body)).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(value["code"], "VALIDATION_ERROR", "{body}");
        assert!(executor.seen.lock().unwrap().is_empty(), "{body}");
    }
}

#[tokio::test]
async fn an_unknown_or_malformed_target_is_refused() {
    let dir = tempdir().unwrap();

    let (app, _) = router(policy(false, dir.path()));
    let (status, value) = send(
        app,
        post_json("/api/v1/live-runs", r#"{"target_id":"missing"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(value["code"], "NOT_FOUND");

    let (app, _) = router(policy(false, dir.path()));
    let (status, value) = send(
        app,
        post_json("/api/v1/live-runs", r#"{"target_id":"Bad Id"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(value["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn override_rules_are_enforced() {
    let dir = tempdir().unwrap();

    let both = r#"{"target_id":"demo","workload":{"requests":5,"duration_seconds":5}}"#;
    let (app, _) = router(policy(false, dir.path()));
    let (status, _) = send(app, post_json("/api/v1/live-runs", both)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let plain_override = r#"{"target_id":"plain","workload":{"requests":5}}"#;
    let (app, _) = router(policy(false, dir.path()));
    let (status, value) = send(app, post_json("/api/v1/live-runs", plain_override)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        value["error"].as_str().unwrap().contains("no workload"),
        "{value}"
    );
}

#[tokio::test]
async fn the_legacy_endpoint_applies_the_same_switch_before_running() {
    let dir = tempdir().unwrap();
    let raw = json!({ "path": dir.path().display().to_string() }).to_string();

    let (app, executor) = router(policy(false, dir.path()));
    let (status, value) = send(app, post_json("/api/v1/benchmarks", &raw)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(value["code"], "FORBIDDEN");
    assert!(executor.seen.lock().unwrap().is_empty());

    let (app, _) = router(policy(false, dir.path()));
    let (status, _) = send(
        app,
        post_json("/api/v1/benchmarks", r#"{"target_id":"missing"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn saved_targets_are_listed_with_names_only() {
    let dir = tempdir().unwrap();
    let (app, _) = router(policy(false, dir.path()));

    let (status, value): (StatusCode, Value) = send(app, get_path("/api/v1/targets")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"]["custom_runs_allowed"], false);

    let targets = value["data"]["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 2);

    let demo = targets
        .iter()
        .find(|target| target["id"] == "demo")
        .unwrap();
    assert_eq!(demo["workload"]["mode"], "duration");
    assert_eq!(demo["workload"]["header_names"], json!(["X-Client-Secret"]));
    assert_eq!(
        demo["startup"]["readiness"]["header_names"],
        json!(["X-Client-Id", "X-Client-Secret"])
    );

    let printed = value.to_string();
    assert!(!printed.contains("http-test-secret"), "{printed}");
}
