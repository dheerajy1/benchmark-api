use std::{
    net::TcpListener,
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use benchmark_api::{
    config::env::Env,
    features::v1::benchmark::{
        error::BenchmarkError,
        live::{LiveRuns, RunContext, RunEntry, RunExecutor, RunPermit, RunSlot, Stage},
        model::{BenchmarkRequest, BenchmarkResult},
    },
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::comparison_sample;

pub fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

// The smallest valid request: a directory, no startup and no workload
pub fn request_for(dir: &Path) -> BenchmarkRequest {
    serde_json::from_value(json!({ "path": dir.display().to_string() })).unwrap()
}

pub fn post_json(body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/benchmarks")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub async fn send(router: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

pub fn kinds(entry: &RunEntry) -> Vec<&'static str> {
    entry
        .replay(0)
        .events
        .iter()
        .map(|event| event.kind)
        .collect()
}

pub async fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);

    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");

        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

pub async fn wait_until_slot_free(slot: &RunSlot) {
    wait_until(|| slot.active().is_none(), "the slot to be free").await;
}

// Waits for the terminal event and checks the guard invariant on every step: the slot
// is released after the terminal event, so a free slot next to a running run can only
// be a violation. The slot is read first so a stale read cannot raise a false alarm.
pub async fn wait_until_terminal(live: &LiveRuns, entry: &RunEntry) {
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        let slot_free = live.slot().active().is_none();
        let terminal = entry.is_terminal();

        assert!(
            !slot_free || terminal,
            "the slot was free while the run was still reported as running"
        );

        if terminal {
            return;
        }

        assert!(Instant::now() < deadline, "the run never became terminal");

        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

// Finishes at once with a complete result
pub struct ImmediateExecutor;

impl RunExecutor for ImmediateExecutor {
    fn execute(
        &self,
        _request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        run_id: Uuid,
        ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        ctx.stage_started(Stage::Build);
        ctx.stage_completed(Stage::Build, "success", Some(1));

        Ok(comparison_sample::result(run_id))
    }
}

// Runs until the test opens the gate
pub struct GatedExecutor {
    pub gate: Arc<AtomicBool>,
}

impl RunExecutor for GatedExecutor {
    fn execute(
        &self,
        _request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        run_id: Uuid,
        ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        ctx.stage_started(Stage::Build);

        while !self.gate.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
        }

        Ok(comparison_sample::result(run_id))
    }
}

pub struct FailingExecutor;

impl RunExecutor for FailingExecutor {
    fn execute(
        &self,
        _request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        _run_id: Uuid,
        ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        ctx.stage_started(Stage::Build);

        Err(BenchmarkError::Internal("boom".to_string()))
    }
}

pub struct PanickingExecutor;

impl RunExecutor for PanickingExecutor {
    fn execute(
        &self,
        _request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        _run_id: Uuid,
        ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        ctx.stage_started(Stage::Startup);

        panic!("executor exploded");
    }
}

pub fn require_bun() {
    let available = Command::new("bun")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    assert!(
        available,
        "bun must be on PATH, the existing runtime tests need it too"
    );
}

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// Explicit startup and a sustained workload, so the run lasts a known time
pub fn long_benchmark_body(dir: &Path, port: u16, seconds: u64) -> String {
    let script = format!("Bun.serve({{ port: {port}, fetch() {{ return new Response('ok'); }} }})");

    json!({
        "path": dir.display().to_string(),
        "startup": {
            "command": { "program": "bun", "args": ["-e", script] },
            "readiness": {
                "type": "http",
                "host": "127.0.0.1",
                "port": port,
                "path": "/",
                "method": "GET",
                "expected_status": 200,
                "headers": {}
            }
        },
        "workload": {
            "method": "GET",
            "path": "/",
            "duration_seconds": seconds,
            "concurrency": 2
        }
    })
    .to_string()
}
