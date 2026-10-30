use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::http::StatusCode;
use benchmark_api::{
    app,
    features::v1::benchmark::{
        live::{
            CancelOutcome, LiveRuns, RunEntry, RunRegistry, RunSlot, ServiceExecutor, event::kind,
            log::RunStatus,
        },
        model::BenchmarkRequest,
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::support::{
    free_port, kinds, post_to, request_for, require_bun, send, test_env, wait_until,
    wait_until_slot_free, wait_until_terminal,
};
use crate::fake_repository::InMemoryRepository;

// Every process of a test carries its own marker in its command line, so the test can see
// whether anything of it is still running
fn processes_with(marker: &str) -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let command_line = fs::read(format!("/proc/{pid}/cmdline")).ok()?;

            String::from_utf8_lossy(&command_line)
                .contains(marker)
                .then_some(pid)
        })
        .collect()
}

fn real_live(repository: Arc<InMemoryRepository>) -> LiveRuns {
    LiveRuns::with_parts(
        test_env(),
        repository,
        RunSlot::new(),
        RunRegistry::new(),
        Arc::new(ServiceExecutor),
    )
}

// A bun project whose build starts two long sleeps
fn build_project(marker: &str) -> TempDir {
    let dir = TempDir::new().unwrap();

    fs::write(dir.path().join("bun.lock"), "").unwrap();
    fs::write(
        dir.path().join("package.json"),
        json!({
            "name": "cancel-fixture",
            "scripts": { "build": format!("sh -c \"sleep {marker} & sleep {marker} & wait\"") }
        })
        .to_string(),
    )
    .unwrap();

    dir
}

fn request_with(dir: &Path, port: u16, shell: &str, workload: Value) -> BenchmarkRequest {
    serde_json::from_value(json!({
        "path": dir.display().to_string(),
        "startup": {
            "command": { "program": "sh", "args": ["-c", shell] },
            "readiness": {
                "type": "http", "host": "127.0.0.1", "port": port,
                "path": "/", "method": "GET", "expected_status": 200, "headers": {}
            }
        },
        "workload": workload
    }))
    .unwrap()
}

// A bun server that answers 200, next to a sleeping process of the same group
fn serving_shell(port: u16, marker: &str) -> String {
    format!(
        "sleep {marker} & exec bun -e 'Bun.serve({{ port: {port}, fetch() {{ return new Response(\"ok\"); }} }})'"
    )
}

fn stage_started(entry: &RunEntry, stage: &str) -> bool {
    entry
        .replay(0)
        .events
        .iter()
        .any(|event| event.kind == kind::STAGE_STARTED && event.data["stage"] == json!(stage))
}

fn has_event(entry: &RunEntry, wanted: &str) -> bool {
    kinds(entry).contains(&wanted)
}

fn last_stage(entry: &RunEntry) -> Value {
    entry.replay(0).events.last().unwrap().data["stage"].clone()
}

async fn cancel_and_wait(live: &LiveRuns, entry: &RunEntry) -> Duration {
    let asked = Instant::now();

    assert_eq!(entry.request_cancel(), CancelOutcome::Requested);

    wait_until_terminal(live, entry).await;

    asked.elapsed()
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_build_stops_it_and_every_process_it_started() {
    require_bun();

    let marker = "9981.5";
    let dir = build_project(marker);
    let repository = Arc::new(InMemoryRepository::default());
    let live = real_live(repository.clone());

    let accepted = live.submit(request_for(dir.path())).expect("accepted");

    wait_until(
        || stage_started(&accepted.entry, "build"),
        "the build to start",
    )
    .await;
    wait_until(
        || processes_with(marker).len() >= 2,
        "the build's processes to run",
    )
    .await;

    let latency = cancel_and_wait(&live, &accepted.entry).await;

    // The terminal event only comes once the whole group is gone
    assert!(
        processes_with(marker).is_empty(),
        "{:?}",
        processes_with(marker)
    );
    assert_eq!(accepted.entry.status(), RunStatus::Cancelled);
    assert_eq!(last_stage(&accepted.entry), json!("build"));
    assert!(latency < Duration::from_secs(5), "{latency:?}");

    wait_until_slot_free(live.slot()).await;

    // The build never finished, so it never reported a result, and nothing was stored
    assert!(!accepted.entry.replay(0).events.iter().any(|event| {
        event.kind == kind::STAGE_COMPLETED && event.data["stage"] == json!("build")
    }));
    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_waiting_for_readiness_stops_the_target_and_its_children() {
    let marker = "9982.5";
    let dir = TempDir::new().unwrap();
    let port = free_port();
    let repository = Arc::new(InMemoryRepository::default());
    let live = real_live(repository.clone());

    // The target never opens its port, so the run waits for readiness
    let request = request_with(
        dir.path(),
        port,
        &format!("sleep {marker} & sleep {marker} & wait"),
        Value::Null,
    );

    let accepted = live.submit(request).expect("accepted");

    wait_until(
        || stage_started(&accepted.entry, "startup"),
        "startup to begin",
    )
    .await;
    wait_until(
        || processes_with(marker).len() >= 2,
        "the target's processes to run",
    )
    .await;

    let latency = cancel_and_wait(&live, &accepted.entry).await;

    assert!(
        processes_with(marker).is_empty(),
        "{:?}",
        processes_with(marker)
    );
    assert_eq!(accepted.entry.status(), RunStatus::Cancelled);
    assert_eq!(last_stage(&accepted.entry), json!("startup"));
    assert!(latency < Duration::from_secs(5), "{latency:?}");

    wait_until_slot_free(live.slot()).await;

    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_the_baseline_stops_the_target_and_its_children() {
    require_bun();

    let marker = "9983.5";
    let dir = TempDir::new().unwrap();
    let port = free_port();
    let repository = Arc::new(InMemoryRepository::default());
    let live = real_live(repository.clone());

    let request = request_with(
        dir.path(),
        port,
        &serving_shell(port, marker),
        json!({ "method": "GET", "path": "/", "duration_seconds": 30, "concurrency": 2 }),
    );

    let accepted = live.submit(request).expect("accepted");

    wait_until(
        || stage_started(&accepted.entry, "baseline"),
        "the baseline to begin",
    )
    .await;

    let latency = cancel_and_wait(&live, &accepted.entry).await;

    assert!(
        processes_with(marker).is_empty(),
        "{:?}",
        processes_with(marker)
    );
    assert_eq!(accepted.entry.status(), RunStatus::Cancelled);
    assert_eq!(last_stage(&accepted.entry), json!("baseline"));
    assert!(!has_event(&accepted.entry, kind::SAMPLE));
    assert!(latency < Duration::from_secs(5), "{latency:?}");

    wait_until_slot_free(live.slot()).await;

    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_during_the_workload_stops_the_requests_the_target_and_its_children() {
    require_bun();

    let marker = "9984.5";
    let dir = TempDir::new().unwrap();
    let port = free_port();
    let repository = Arc::new(InMemoryRepository::default());
    let live = real_live(repository.clone());

    let request = request_with(
        dir.path(),
        port,
        &serving_shell(port, marker),
        json!({ "method": "GET", "path": "/", "duration_seconds": 30, "concurrency": 4 }),
    );

    let accepted = live.submit(request).expect("accepted");

    wait_until(
        || has_event(&accepted.entry, kind::SAMPLE),
        "the first sample",
    )
    .await;

    let latency = cancel_and_wait(&live, &accepted.entry).await;

    assert!(
        processes_with(marker).is_empty(),
        "{:?}",
        processes_with(marker)
    );
    assert_eq!(accepted.entry.status(), RunStatus::Cancelled);
    assert_eq!(last_stage(&accepted.entry), json!("workload"));

    // Stopped long before the 30 seconds of the workload were over
    assert!(latency < Duration::from_secs(5), "{latency:?}");

    // The workload never reported a result, and no sample can follow the end
    let replay = accepted.entry.replay(0);

    assert!(!replay.events.iter().any(|event| {
        event.kind == kind::STAGE_COMPLETED && event.data["stage"] == json!("workload")
    }));
    assert_eq!(replay.events.last().unwrap().kind, kind::RUN_CANCELLED);

    let samples_at_the_end = replay
        .events
        .iter()
        .filter(|e| e.kind == kind::SAMPLE)
        .count();

    tokio::time::sleep(Duration::from_millis(1200)).await;

    let samples_later = accepted
        .entry
        .replay(0)
        .events
        .iter()
        .filter(|e| e.kind == kind::SAMPLE)
        .count();

    assert_eq!(samples_at_the_end, samples_later);

    wait_until_slot_free(live.slot()).await;

    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_before_anything_started_leaves_nothing_running() {
    require_bun();

    let marker = "9985.5";
    let dir = build_project(marker);
    let repository = Arc::new(InMemoryRepository::default());
    let live = real_live(repository.clone());

    let accepted = live.submit(request_for(dir.path())).expect("accepted");

    accepted.entry.request_cancel();

    wait_until_terminal(&live, &accepted.entry).await;

    assert!(
        processes_with(marker).is_empty(),
        "{:?}",
        processes_with(marker)
    );
    assert_eq!(accepted.entry.status(), RunStatus::Cancelled);

    wait_until_slot_free(live.slot()).await;

    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_run_that_finishes_normally_leaves_no_process_behind() {
    require_bun();

    let marker = "9986.5";
    let dir = TempDir::new().unwrap();
    let port = free_port();
    let repository = Arc::new(InMemoryRepository::default());
    let live = real_live(repository.clone());

    let request = request_with(
        dir.path(),
        port,
        &serving_shell(port, marker),
        json!({ "method": "GET", "path": "/", "duration_seconds": 1, "concurrency": 2 }),
    );

    let accepted = live.submit(request).expect("accepted");

    wait_until_terminal(&live, &accepted.entry).await;

    // The sleeping sibling of the server is part of the group and must be gone too
    assert!(
        processes_with(marker).is_empty(),
        "{:?}",
        processes_with(marker)
    );
    assert_eq!(accepted.entry.status(), RunStatus::Completed);

    wait_until_slot_free(live.slot()).await;

    assert_eq!(repository.saved_run_ids(), vec![accepted.run_id]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_legacy_run_also_leaves_no_process_behind_and_answers_as_before() {
    require_bun();

    let marker = "9987.5";
    let dir = TempDir::new().unwrap();
    let port = free_port();
    let repository = Arc::new(InMemoryRepository::default());
    let live = real_live(repository.clone());
    let router = app::router_with_live(test_env(), repository.clone(), live.clone());

    let body = json!({
        "path": dir.path().display().to_string(),
        "startup": {
            "command": { "program": "sh", "args": ["-c", serving_shell(port, marker)] },
            "readiness": {
                "type": "http", "host": "127.0.0.1", "port": port,
                "path": "/", "method": "GET", "expected_status": 200, "headers": {}
            }
        },
        "workload": { "method": "GET", "path": "/", "duration_seconds": 1, "concurrency": 2 }
    })
    .to_string();

    let (status, response) = send(router, post_to("/api/v1/benchmarks", &body)).await;

    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["message"], json!("Benchmark completed."));
    assert!(
        processes_with(marker).is_empty(),
        "{:?}",
        processes_with(marker)
    );
    assert!(live.slot().active().is_none());
    assert_eq!(repository.saved_run_ids().len(), 1);
}
