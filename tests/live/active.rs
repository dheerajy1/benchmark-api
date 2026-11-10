use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use axum::{Router, http::StatusCode};
use benchmark_api::{
    app,
    features::v1::benchmark::live::{LiveRuns, RunRegistry, RunSlot},
};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::support::{
    GatedExecutor, get_json, post_to, send, test_env, wait_until, wait_until_slot_free,
};
use crate::fake_repository::InMemoryRepository;

const URI: &str = "/api/v1/live-runs/active";

fn router_with_gate(gate: &Arc<AtomicBool>) -> (Router, LiveRuns) {
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::with_parts(
        test_env(),
        repository.clone(),
        RunSlot::new(),
        RunRegistry::new(),
        Arc::new(GatedExecutor {
            gate: Arc::clone(gate),
        }),
    );

    (
        app::router_with_live(test_env(), repository, live.clone()),
        live,
    )
}

async fn active(router: &Router) -> Value {
    let (status, body) = send(router.clone(), get_json(URI)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["success"], json!(true));

    body["data"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_running_is_a_null_data_answer() {
    let (router, _live) = router_with_gate(&Arc::new(AtomicBool::new(true)));

    assert_eq!(active(&router).await, json!(null));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_run_is_described_and_a_finished_run_is_not() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));
    let (router, live) = router_with_gate(&gate);

    let body = json!({ "path": dir.path().display().to_string(), "build": { "command": "true", "output_dir": "." } }).to_string();
    let (status, accepted) = send(router.clone(), post_to("/api/v1/live-runs", &body)).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");

    let run_id = Uuid::parse_str(accepted["data"]["run_id"].as_str().unwrap()).unwrap();

    wait_until(
        || {
            live.registry()
                .get(run_id)
                .is_some_and(|entry| entry.current_stage().is_some())
        },
        "the run to start its first stage",
    )
    .await;

    let data = active(&router).await;

    assert_eq!(data["run_id"], json!(run_id.to_string()));
    assert_eq!(data["stage"], json!("build"));
    assert!(data["elapsed_ms"].is_u64());
    // No workload sample yet, so nothing is invented
    assert_eq!(data["workload_progress"], json!(null));

    gate.store(true, Ordering::SeqCst);
    wait_until_slot_free(live.slot()).await;

    assert_eq!(active(&router).await, json!(null));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_workload_progress_is_the_newest_sample_progress() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));
    let (router, live) = router_with_gate(&gate);

    let body = json!({ "path": dir.path().display().to_string(), "build": { "command": "true", "output_dir": "." } }).to_string();
    let (_, accepted) = send(router.clone(), post_to("/api/v1/live-runs", &body)).await;
    let run_id = Uuid::parse_str(accepted["data"]["run_id"].as_str().unwrap()).unwrap();

    wait_until(
        || live.registry().get(run_id).is_some(),
        "the run to be registered",
    )
    .await;

    let entry = live.registry().get(run_id).unwrap();

    for done in [10, 20] {
        entry
            .append(
                "sample",
                json!({ "http": { "progress": { "mode": "requests", "done": done, "total": 100 } } }),
            )
            .unwrap();
    }

    assert_eq!(
        active(&router).await["workload_progress"],
        json!({ "mode": "requests", "done": 20, "total": 100 })
    );

    gate.store(true, Ordering::SeqCst);
    wait_until_slot_free(live.slot()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_active_route_rejects_other_methods_and_keeps_the_run_id_route() {
    let (router, _live) = router_with_gate(&Arc::new(AtomicBool::new(true)));

    let (status, body) = send(router.clone(), post_to(URI, "{}")).await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body["code"], json!("METHOD_NOT_ALLOWED"));

    let (status, body) = send(router, get_json("/api/v1/live-runs/not-a-uuid")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
