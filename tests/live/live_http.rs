use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use axum::{Router, http::StatusCode};
use benchmark_api::{
    app,
    features::v1::benchmark::live::{LiveRuns, RunExecutor, RunRegistry, RunSlot},
};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::support::{
    GatedExecutor, ImmediateExecutor, get_json, post_to, send, test_env, wait_until,
    wait_until_slot_free,
};
use crate::fake_repository::InMemoryRepository;

fn router_with(executor: Arc<dyn RunExecutor>) -> (Router, LiveRuns) {
    let repository = Arc::new(InMemoryRepository::default());

    let live = LiveRuns::with_parts(
        test_env(),
        repository.clone(),
        RunSlot::new(),
        RunRegistry::new(),
        executor,
    );

    let router = app::router_with_live(test_env(), repository, live.clone());

    (router, live)
}

fn valid_body(dir: &TempDir) -> String {
    json!({ "path": dir.path().display().to_string() }).to_string()
}

fn run_id_of(body: &Value) -> Uuid {
    Uuid::parse_str(body["data"]["run_id"].as_str().unwrap()).unwrap()
}

fn assert_error(body: &Value, http_code: u64, code: &str) {
    assert_eq!(body["success"], json!(false), "{body}");
    assert_eq!(body["httpCode"], json!(http_code));
    assert_eq!(body["code"], json!(code));
    assert!(body["message"].is_string());
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_submission_is_accepted_with_the_run_id_and_the_paths_to_follow_it() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let (router, live) = router_with(Arc::new(GatedExecutor {
        gate: Arc::clone(&gate),
    }));

    let (status, body) = send(
        router.clone(),
        post_to("/api/v1/live-runs", &valid_body(&dir)),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["httpCode"], json!(202));
    assert_eq!(body["message"], json!("Benchmark accepted."));

    let run_id = run_id_of(&body);

    assert_eq!(
        body["data"]["ws_path"],
        json!(format!("/api/v1/live-runs/{run_id}/ws"))
    );
    assert_eq!(
        body["data"]["status_path"],
        json!(format!("/api/v1/live-runs/{run_id}"))
    );
    assert_eq!(body["data"]["capabilities"], json!({ "cancel": false }));

    // The run is registered before the 202 is sent
    let (status, replay) = send(
        router.clone(),
        get_json(body["data"]["status_path"].as_str().unwrap()),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["data"]["status"], json!("running"));
    assert_eq!(replay["data"]["run_id"], json!(run_id.to_string()));
    assert_eq!(replay["data"]["truncated"], json!(false));
    assert_eq!(replay["data"]["dropped_through_seq"], json!(0));
    assert_eq!(replay["data"]["events"][0]["seq"], json!(1));
    assert_eq!(replay["data"]["events"][0]["type"], json!("run.accepted"));

    gate.store(true, Ordering::SeqCst);

    wait_until(
        || live.registry_status(run_id) != Some("running"),
        "the run to finish",
    )
    .await;
    wait_until_slot_free(live.slot()).await;

    let (_status, replay) = send(
        router.clone(),
        get_json(&format!("/api/v1/live-runs/{run_id}")),
    )
    .await;

    assert_eq!(replay["data"]["status"], json!("completed"));

    let events = replay["data"]["events"].as_array().unwrap();

    assert_eq!(events.last().unwrap()["type"], json!("run.completed"));
    assert_eq!(events.last().unwrap()["data"]["persisted"], json!(true));

    // After a sequence only the later events are returned
    let last_seq = replay["data"]["last_seq"].as_u64().unwrap();

    let (_status, later) = send(
        router,
        get_json(&format!(
            "/api/v1/live-runs/{run_id}?after={}",
            last_seq - 1
        )),
    )
    .await;

    assert_eq!(later["data"]["events"].as_array().unwrap().len(), 1);
    assert_eq!(later["data"]["status"], json!("completed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_submission_gets_409_with_the_active_run_id() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let (router, live) = router_with(Arc::new(GatedExecutor {
        gate: Arc::clone(&gate),
    }));

    let (_status, first) = send(
        router.clone(),
        post_to("/api/v1/live-runs", &valid_body(&dir)),
    )
    .await;

    let (status, second) = send(
        router.clone(),
        post_to("/api/v1/live-runs", &valid_body(&dir)),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&second, 409, "CONFLICT");
    assert_eq!(second["error"], json!(run_id_of(&first).to_string()));

    gate.store(true, Ordering::SeqCst);

    wait_until_slot_free(live.slot()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_submission_is_a_400_and_takes_no_slot() {
    let (router, live) = router_with(Arc::new(ImmediateExecutor));

    let (status, body) = send(
        router.clone(),
        post_to(
            "/api/v1/live-runs",
            r#"{"path":"/definitely/not/a/real/directory"}"#,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, 400, "BAD_REQUEST");
    assert!(live.slot().active().is_none());
    assert!(live.registry().is_empty());

    let (status, body) = send(router, post_to("/api/v1/live-runs", "{ not json")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, 400, "BAD_REQUEST");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_status_route_reports_unknown_and_malformed_requests_in_the_error_shape() {
    let (router, _live) = router_with(Arc::new(ImmediateExecutor));

    let (status, body) = send(
        router.clone(),
        get_json(&format!("/api/v1/live-runs/{}", Uuid::now_v7())),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&body, 404, "NOT_FOUND");

    let (status, body) = send(router.clone(), get_json("/api/v1/live-runs/not-a-uuid")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, 400, "BAD_REQUEST");

    let (status, body) = send(
        router,
        get_json(&format!("/api/v1/live-runs/{}?after=abc", Uuid::now_v7())),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, 400, "VALIDATION_ERROR");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_live_run_routes_refuse_other_methods() {
    let (router, _live) = router_with(Arc::new(ImmediateExecutor));

    let (status, body) = send(router.clone(), get_json("/api/v1/live-runs")).await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_error(&body, 405, "METHOD_NOT_ALLOWED");

    let (status, body) = send(
        router,
        post_to(&format!("/api/v1/live-runs/{}", Uuid::now_v7()), "{}"),
    )
    .await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_error(&body, 405, "METHOD_NOT_ALLOWED");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_get_on_the_socket_route_is_not_an_upgrade() {
    let (router, _live) = router_with(Arc::new(ImmediateExecutor));

    let response = tower::ServiceExt::oneshot(
        router,
        get_json(&format!("/api/v1/live-runs/{}/ws", Uuid::now_v7())),
    )
    .await
    .unwrap();

    assert!(response.status().is_client_error(), "{}", response.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_legacy_endpoint_and_the_history_routes_are_unchanged_by_the_new_routes() {
    let dir = TempDir::new().unwrap();
    let (router, _live) = router_with(Arc::new(ImmediateExecutor));

    let (status, body) = send(
        router.clone(),
        post_to("/api/v1/benchmarks", &valid_body(&dir)),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], json!("Benchmark completed."));

    let run_id = body["data"]["run_id"].as_str().unwrap().to_string();

    let (status, _body) = send(router, get_json(&format!("/api/v1/benchmarks/{run_id}"))).await;

    // The in-memory repository of these tests keeps the run, the route still answers
    assert!(status == StatusCode::OK || status == StatusCode::NOT_FOUND);
}
