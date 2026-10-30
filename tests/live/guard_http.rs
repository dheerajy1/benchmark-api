use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use benchmark_api::{
    app,
    features::v1::benchmark::live::{
        ActiveRun, LiveRuns, RunKind, RunRegistry, RunSlot, SubmitError,
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

use super::support::{
    GatedExecutor, ImmediateExecutor, free_port, long_benchmark_body, post_json, request_for,
    require_bun, send, test_env, wait_until, wait_until_slot_free, wait_until_terminal,
};
use crate::fake_repository::{FailingRepository, InMemoryRepository};

fn assert_error_shape(body: &Value, http_code: u64, code: &str) {
    assert_eq!(body["success"], json!(false));
    assert_eq!(body["httpCode"], json!(http_code));
    assert_eq!(body["code"], json!(code));
    assert!(body["message"].is_string());
    assert!(body["error"].is_string());
}

fn valid_body(dir: &std::path::Path) -> String {
    json!({ "path": dir.display().to_string() }).to_string()
}

#[tokio::test]
async fn a_slot_held_by_a_live_run_refuses_the_legacy_endpoint_and_names_the_run() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository.clone(), live.clone());

    let run_id = uuid::Uuid::now_v7();

    let _permit = live
        .slot()
        .try_acquire(RunKind::Live, Some(run_id))
        .expect("free slot");

    let (status, body) = send(router, post_json(&valid_body(temp_dir.path()))).await;

    assert_eq!(status, 409);
    assert_error_shape(&body, 409, "CONFLICT");
    assert_eq!(body["error"], json!(run_id.to_string()));
    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test]
async fn a_slot_held_by_a_legacy_run_refuses_the_legacy_endpoint_with_a_plain_detail() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository, live.clone());

    let _permit = live
        .slot()
        .try_acquire(RunKind::Legacy, None)
        .expect("free slot");

    let (status, body) = send(router, post_json(&valid_body(temp_dir.path()))).await;

    assert_eq!(status, 409);
    assert_error_shape(&body, 409, "CONFLICT");
    assert_eq!(body["error"], json!("A benchmark is already running."));
}

#[tokio::test]
async fn an_invalid_request_is_still_a_400_while_the_slot_is_busy() {
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository, live.clone());

    let _permit = live
        .slot()
        .try_acquire(RunKind::Live, Some(uuid::Uuid::now_v7()))
        .expect("free slot");

    let (status, body) = send(
        router,
        post_json(r#"{"path":"/definitely/not/a/real/directory"}"#),
    )
    .await;

    assert_eq!(status, 400);
    assert_error_shape(&body, 400, "BAD_REQUEST");
}

#[tokio::test]
async fn a_legacy_request_releases_the_slot_when_it_finishes() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository.clone(), live.clone());

    let (status, body) = send(router, post_json(&valid_body(temp_dir.path()))).await;

    assert_eq!(status, 200);
    assert_eq!(body["message"], json!("Benchmark completed."));
    assert!(live.slot().active().is_none());
    assert_eq!(repository.saved_run_ids().len(), 1);
}

#[tokio::test]
async fn a_legacy_request_that_cannot_persist_still_releases_the_slot() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(FailingRepository);
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository, live.clone());

    let (status, body) = send(router, post_json(&valid_body(temp_dir.path()))).await;

    assert_eq!(status, 500);
    assert_error_shape(&body, 500, "INTERNAL_SERVER_ERROR");
    assert!(live.slot().active().is_none());
}

#[tokio::test]
async fn a_legacy_request_with_a_missing_path_leaves_the_slot_free() {
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository, live.clone());

    let (status, _body) = send(
        router,
        post_json(r#"{"path":"/definitely/not/a/real/directory"}"#),
    )
    .await;

    assert_eq!(status, 400);
    assert!(live.slot().active().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_run_in_flight_refuses_the_legacy_endpoint_until_it_ends() {
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

    assert_eq!(status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_immediate_live_run_followed_by_a_legacy_request_both_succeed() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());

    let live = LiveRuns::with_parts(
        test_env(),
        repository.clone(),
        RunSlot::new(),
        RunRegistry::new(),
        Arc::new(ImmediateExecutor),
    );

    let router = app::router_with_live(test_env(), repository.clone(), live.clone());

    let accepted = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    let (status, _body) = send(router, post_json(&valid_body(temp_dir.path()))).await;

    assert_eq!(status, 200);
    assert_eq!(repository.saved_run_ids().len(), 2);
}

// The key requirement: when the client of a legacy request disappears, the benchmark
// keeps running and the slot is only released when the benchmark itself has finished.
// The request task is aborted to drop the handler future, which is what hyper does on a
// disconnect (shown by the Phase 0 tests).
#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_legacy_request_keeps_the_slot_until_the_benchmark_has_finished() {
    require_bun();

    let temp_dir = TempDir::new().unwrap();
    let port = free_port();
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());
    let router = app::router_with_live(test_env(), repository.clone(), live.clone());

    let request = post_json(&long_benchmark_body(temp_dir.path(), port, 2));

    let handle = tokio::spawn(async move { router.oneshot(request).await });

    wait_until(
        || live.slot().active().is_some(),
        "the legacy run to take the slot",
    )
    .await;

    handle.abort();
    let _ = handle.await;

    assert_eq!(
        live.slot().active(),
        Some(ActiveRun {
            kind: RunKind::Legacy,
            run_id: None
        }),
        "the slot was released when the client went away"
    );

    // Still executing well after the handler is gone, a 2 s workload cannot be over yet
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        live.slot().active().is_some(),
        "the slot was released while the benchmark was still running"
    );

    // A live submission is refused for the whole time
    let refused = live
        .submit(request_for(temp_dir.path()))
        .err()
        .expect("refused while the orphaned legacy run executes");

    assert_eq!(
        refused,
        SubmitError::Busy(ActiveRun {
            kind: RunKind::Legacy,
            run_id: None
        })
    );

    wait_until_slot_free(live.slot()).await;

    // Existing behavior, deliberately unchanged in Phase 1: the dropped request's
    // result is not saved
    assert!(repository.saved_run_ids().is_empty());
}
