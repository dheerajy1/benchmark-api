use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use benchmark_api::{
    app,
    features::v1::benchmark::live::{LiveRuns, RunKind, RunRegistry, RunSlot},
};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::support::{
    GatedExecutor, ImmediateExecutor, post_json, request_for, send, test_env, wait_until_slot_free,
    wait_until_terminal,
};
use crate::fake_repository::InMemoryRepository;

fn assert_error_shape(body: &Value, http_code: u64, code: &str) {
    assert_eq!(body["success"], json!(false));
    assert_eq!(body["httpCode"], json!(http_code));
    assert_eq!(body["code"], json!(code));
    assert!(body["message"].is_string());
    assert!(body["error"].is_string());
}

fn valid_body(dir: &std::path::Path) -> String {
    json!({ "path": dir.display().to_string(), "build": { "command": "true", "output_dir": "." } })
        .to_string()
}

#[tokio::test]
async fn a_slot_held_by_a_run_refuses_a_new_run_and_names_it() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository.clone(), live.clone());

    let run_id = uuid::Uuid::now_v7();

    let _permit = live
        .slot()
        .try_acquire(RunKind::Live, run_id)
        .expect("free slot");

    let (status, body) = send(router, post_json(&valid_body(temp_dir.path()))).await;

    assert_eq!(status, 409);
    assert_error_shape(&body, 409, "CONFLICT");
    assert_eq!(body["error"], json!(run_id.to_string()));
    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test]
async fn an_invalid_request_is_still_a_400_while_the_slot_is_busy() {
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository, live.clone());

    let _permit = live
        .slot()
        .try_acquire(RunKind::Live, uuid::Uuid::now_v7())
        .expect("free slot");

    let (status, body) = send(
        router,
        post_json(r#"{"path":"/definitely/not/a/real/directory","build":{"command":"true"}}"#),
    )
    .await;

    assert_eq!(status, 400);
    assert_error_shape(&body, 400, "BAD_REQUEST");
}

#[tokio::test]
async fn a_request_with_a_missing_path_leaves_the_slot_free() {
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository, live.clone());

    let (status, _body) = send(
        router,
        post_json(r#"{"path":"/definitely/not/a/real/directory","build":{"command":"true"}}"#),
    )
    .await;

    assert_eq!(status, 400);
    assert!(live.slot().active().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_in_flight_refuses_a_new_run_until_it_ends() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());
    let gate = Arc::new(AtomicBool::new(false));

    let live = LiveRuns::with_parts(
        test_env(),
        repository.clone(),
        RunSlot::new(),
        RunRegistry::new(),
        Arc::new(GatedExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let router = app::router_with_live(test_env(), repository.clone(), live.clone());

    let accepted = live.submit(request_for(temp_dir.path())).expect("accepted");

    let (status, body) = send(router.clone(), post_json(&valid_body(temp_dir.path()))).await;

    assert_eq!(status, 409);
    assert_eq!(body["error"], json!(accepted.run_id.to_string()));

    gate.store(true, Ordering::SeqCst);

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    let (status, _body) = send(router, post_json(&valid_body(temp_dir.path()))).await;

    assert_eq!(status, 202);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_runs_one_after_the_other_are_both_stored() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());

    let live = LiveRuns::with_parts(
        test_env(),
        repository.clone(),
        RunSlot::new(),
        RunRegistry::new(),
        Arc::new(ImmediateExecutor),
    );

    let first = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until_terminal(&live, &first.entry).await;
    wait_until_slot_free(live.slot()).await;

    let second = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until_terminal(&live, &second.entry).await;
    wait_until_slot_free(live.slot()).await;

    assert_eq!(repository.saved_run_ids().len(), 2);
}
