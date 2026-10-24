use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use benchmark_api::{
    app,
    config::env::Env,
    features::v1::{
        benchmark::{
            error::BenchmarkError,
            live::{LiveRuns, RunContext, RunExecutor, RunPermit, RunRegistry, RunSlot},
            model::{BenchmarkRequest, BenchmarkResult},
        },
        targets::policy::RunPolicy,
    },
    persistence::repository::BenchmarkRepository,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::fake_repository::InMemoryRepository;

pub fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

// A lookup over fixed pairs, standing in for the process environment
pub fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();

    move |name: &str| {
        owned
            .iter()
            .find(|(key, _)| key.as_str() == name)
            .map(|(_, value)| value.clone())
    }
}

// Two targets: one with startup and a workload, one with neither
pub fn targets_json(dir: &Path) -> String {
    json!({
        "targets": {
            "demo": {
                "name": "Demo API",
                "path": dir.display().to_string(),
                "startup": {
                    "command": { "program": "bun", "args": ["run", "dev"] },
                    "readiness": {
                        "port": 5000,
                        "path": "/health",
                        "expected_status": 200,
                        "headers": {
                            "X-Client-Id": "demo-client",
                            "X-Client-Secret": { "env": "DEMO_SECRET" }
                        }
                    }
                },
                "workload": {
                    "method": "GET",
                    "path": "/health",
                    "duration_seconds": 30,
                    "concurrency": 50,
                    "headers": { "X-Client-Secret": { "env": "DEMO_SECRET" } }
                }
            },
            "plain": {
                "name": "Plain API",
                "path": dir.display().to_string()
            }
        }
    })
    .to_string()
}

pub fn write_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

// Records every request it is given, then fails the run so no repository is touched
#[derive(Default)]
pub struct RecordingExecutor {
    pub seen: Mutex<Vec<BenchmarkRequest>>,
}

impl RunExecutor for RecordingExecutor {
    fn execute(
        &self,
        request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        _run_id: Uuid,
        _ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        self.seen.lock().unwrap().push(request.clone());

        Err(BenchmarkError::Internal(
            "recorded by the test executor".to_string(),
        ))
    }
}

// The full router with a policy, and the executor that records what it was asked to run
pub fn router(policy: RunPolicy) -> (Router, Arc<RecordingExecutor>) {
    let executor = Arc::new(RecordingExecutor::default());
    let repository: Arc<dyn BenchmarkRepository> = Arc::new(InMemoryRepository::default());

    let live = LiveRuns::with_parts(
        test_env(),
        Arc::clone(&repository),
        RunSlot::new(),
        RunRegistry::new(),
        executor.clone(),
    );

    let router = app::router_with_policy(test_env(), repository, live, policy);

    (router, executor)
}

// Waits until the executor has seen the given number of requests
pub async fn wait_for_seen(executor: &RecordingExecutor, count: usize) {
    for _ in 0..400 {
        if executor.seen.lock().unwrap().len() >= count {
            return;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    panic!("the executor did not receive {count} request(s)");
}

pub fn post_raw(path: &str, body: &str, content_type: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(Method::POST).uri(path);

    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }

    builder.body(Body::from(body.to_string())).unwrap()
}

pub fn post_json(path: &str, body: &str) -> Request<Body> {
    post_raw(path, body, Some("application/json"))
}

pub fn get_path(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

pub async fn send(router: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

// Shared by the HTTP tests: a policy over the fixture targets file in the given directory
pub fn policy(custom_runs_allowed: bool, dir: &Path) -> RunPolicy {
    let file = write_file(dir, "targets.json", &targets_json(dir));
    let lookup = lookup(&[("DEMO_SECRET", "http-test-secret")]);
    let catalog =
        benchmark_api::features::v1::targets::catalog::TargetCatalog::load(&file, &lookup)
            .expect("the fixture targets file loads");

    RunPolicy::new(custom_runs_allowed, catalog)
}
