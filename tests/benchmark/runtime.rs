use std::{net::TcpListener, path::Path, thread, time::Duration};

use benchmark_api::config::env::Env;
use benchmark_api::features::v1::benchmark::{
    metrics::memory::MemoryConfig,
    model::BenchmarkRequest,
    runtime::{self, RuntimeConfig},
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

fn fast_config() -> RuntimeConfig {
    RuntimeConfig {
        memory: MemoryConfig {
            stabilization: Duration::from_millis(50),
            sample_interval: Duration::from_millis(20),
        },
        request_timeout: Duration::from_secs(5),
    }
}

fn serve_script(port: u16) -> String {
    format!("Bun.serve({{ port: {port}, fetch() {{ return new Response('ok'); }} }})")
}

// Server that exits on its own shortly after starting
fn dying_script(port: u16, lifetime_ms: u64) -> String {
    format!(
        "Bun.serve({{ port: {port}, fetch() {{ return new Response('ok'); }} }}); setTimeout(() => process.exit(1), {lifetime_ms})"
    )
}

fn workload(requests: u64, concurrency: u64) -> Value {
    json!({
        "method": "GET",
        "path": "/",
        "headers": { "X-Test": "1" },
        "requests": requests,
        "concurrency": concurrency
    })
}

fn request(dir: &Path, script: &str, port: u16, workload: Value) -> BenchmarkRequest {
    serde_json::from_value(json!({
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
        "workload": workload
    }))
    .unwrap()
}

fn assert_port_released(port: u16) {
    thread::sleep(Duration::from_millis(300));

    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "target still holds port {port}"
    );
}

#[test]
fn runs_workload_against_a_ready_target_and_cleans_up() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let request = request(temp_dir.path(), &serve_script(port), port, workload(60, 6));

    let metrics = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    )
    .unwrap();

    assert_eq!(metrics.startup_time.status, "success");

    let latency = metrics.http_latency.unwrap();

    assert_eq!(latency.status, "completed");
    assert_eq!(latency.total_requests, 60);
    assert_eq!(latency.successful_requests, 60);
    assert_eq!(latency.failed_requests, 0);
    assert!(latency.requests_per_second > 0.0);
    assert!(latency.min_ms.unwrap() <= latency.max_ms.unwrap());

    let memory = metrics.memory.unwrap();

    assert!(memory.baseline.rss_bytes > 0);
    assert!(memory.peak.rss_bytes >= memory.baseline.rss_bytes);
    assert_eq!(
        memory.delta.rss_bytes,
        memory.peak.rss_bytes - memory.baseline.rss_bytes
    );

    assert_port_released(port);
}

#[test]
fn without_a_workload_runtime_metrics_are_null_and_target_is_shut_down() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let request = request(temp_dir.path(), &serve_script(port), port, Value::Null);

    let metrics = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    )
    .unwrap();

    assert_eq!(metrics.startup_time.status, "success");
    assert!(metrics.memory.is_none());
    assert!(metrics.http_latency.is_none());

    assert_port_released(port);
}

#[test]
fn failed_startup_skips_the_workload() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let request = request(temp_dir.path(), "process.exit(3)", port, workload(10, 2));

    let metrics = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    )
    .unwrap();

    assert_eq!(metrics.startup_time.status, "failed");
    assert_eq!(metrics.startup_time.exit_code, Some(3));
    assert!(metrics.memory.is_none());
    assert!(metrics.http_latency.is_none());
}

#[test]
fn unhealthy_startup_skips_the_workload_and_shuts_down() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let script = format!(
        "Bun.serve({{ port: {port}, fetch() {{ return new Response('no', {{ status: 500 }}); }} }})"
    );

    let request = request(temp_dir.path(), &script, port, workload(10, 2));

    let metrics = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    )
    .unwrap();

    assert_eq!(metrics.startup_time.status, "unhealthy");
    assert!(metrics.memory.is_none());
    assert!(metrics.http_latency.is_none());

    assert_port_released(port);
}

#[test]
fn non_2xx_workload_responses_are_failed_requests_not_benchmark_errors() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    // Readiness path "/" is healthy, the workload path answers 404
    let script = format!(
        "Bun.serve({{ port: {port}, fetch(req) {{ return new URL(req.url).pathname === '/' ? new Response('ok') : new Response('missing', {{ status: 404 }}); }} }})"
    );

    let request = request(
        temp_dir.path(),
        &script,
        port,
        json!({
            "method": "GET",
            "path": "/missing",
            "requests": 20,
            "concurrency": 4
        }),
    );

    let metrics = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    )
    .unwrap();

    let latency = metrics.http_latency.unwrap();

    assert_eq!(latency.status, "completed");
    assert_eq!(latency.total_requests, 20);
    assert_eq!(latency.successful_requests, 0);
    assert_eq!(latency.failed_requests, 20);
    assert!(metrics.memory.is_some());

    assert_port_released(port);
}

#[test]
fn target_exit_during_the_workload_returns_partial_metrics() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let request = request(
        temp_dir.path(),
        &dying_script(port, 1500),
        port,
        workload(5_000_000, 4),
    );

    let metrics = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    )
    .unwrap();

    assert_eq!(metrics.startup_time.status, "success");

    let latency = metrics.http_latency.unwrap();

    assert_eq!(latency.status, "target_exited");
    assert!(latency.total_requests > 0);
    assert!(latency.total_requests < 5_000_000);
    assert_eq!(
        latency.total_requests,
        latency.successful_requests + latency.failed_requests
    );

    let memory = metrics.memory.unwrap();

    assert!(memory.baseline.rss_bytes > 0);
    assert!(memory.peak.rss_bytes >= memory.baseline.rss_bytes);

    assert_port_released(port);
}

#[test]
fn invalid_workload_at_runtime_still_shuts_the_target_down() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    // Bypasses service validation on purpose to exercise the error path
    let request = request(
        temp_dir.path(),
        &serve_script(port),
        port,
        json!({
            "method": "GET",
            "path": "no-leading-slash",
            "requests": 5,
            "concurrency": 1
        }),
    );

    let result = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    );

    assert!(result.is_err());

    assert_port_released(port);
}
