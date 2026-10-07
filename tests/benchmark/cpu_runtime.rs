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

// Sample interval stays above the minimum interval that counts toward the CPU peak
fn fast_config() -> RuntimeConfig {
    RuntimeConfig {
        memory: MemoryConfig {
            stabilization: Duration::from_millis(50),
            sample_interval: Duration::from_millis(60),
        },
        request_timeout: Duration::from_secs(5),
    }
}

fn serve_script(port: u16) -> String {
    format!("Bun.serve({{ port: {port}, fetch() {{ return new Response('ok'); }} }})")
}

// Readiness path "/" answers immediately, "/burn" keeps the target busy for 10 ms
fn busy_script(port: u16) -> String {
    "Bun.serve({ port: PORT, fetch(req) { if (new URL(req.url).pathname === '/') return new Response('ok'); const end = Date.now() + 10; while (Date.now() < end) {} return new Response('burned'); } })"
        .replace("PORT", &port.to_string())
}

fn dying_script(port: u16, lifetime_ms: u64) -> String {
    format!(
        "Bun.serve({{ port: {port}, fetch() {{ return new Response('ok'); }} }}); setTimeout(() => process.exit(1), {lifetime_ms})"
    )
}

fn duration_workload(path: &str, seconds: u64, concurrency: u64) -> Value {
    json!({
        "method": "GET",
        "path": path,
        "duration_seconds": seconds,
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
fn sustained_workload_reports_cpu_alongside_memory() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let request = request(
        temp_dir.path(),
        &serve_script(port),
        port,
        duration_workload("/", 1, 4),
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

    assert_eq!(latency.status, "completed");
    assert!(latency.duration_ms >= 1000);

    let memory = metrics.memory.unwrap();

    assert!(memory.baseline.rss_bytes > 0);

    let cpu = metrics.cpu.unwrap();

    assert!(cpu.baseline_percent >= 0.0);
    assert!(cpu.average_percent > 0.0);
    assert!(cpu.peak_percent >= cpu.average_percent);

    assert_port_released(port);
}

#[test]
fn busy_target_shows_measurable_cpu_usage() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let request = request(
        temp_dir.path(),
        &busy_script(port),
        port,
        duration_workload("/burn", 1, 4),
    );

    let metrics = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    )
    .unwrap();

    let cpu = metrics.cpu.unwrap();

    assert!(cpu.average_percent > 10.0, "cpu was {cpu:?}");
    assert!(cpu.peak_percent >= cpu.average_percent);
    assert!(cpu.average_percent > cpu.baseline_percent);

    assert_port_released(port);
}

#[test]
fn without_a_workload_cpu_is_null() {
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
    assert!(metrics.cpu.is_none());
    assert!(metrics.memory.is_none());
    assert!(metrics.http_latency.is_none());

    assert_port_released(port);
}

#[test]
fn failed_startup_leaves_cpu_null() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let request = request(
        temp_dir.path(),
        "process.exit(3)",
        port,
        duration_workload("/", 1, 2),
    );

    let metrics = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    )
    .unwrap();

    assert_eq!(metrics.startup_time.status, "failed");
    assert!(metrics.cpu.is_none());
    assert!(metrics.memory.is_none());
    assert!(metrics.http_latency.is_none());
}

#[test]
fn unhealthy_startup_leaves_cpu_null_and_shuts_down() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let script = format!(
        "Bun.serve({{ port: {port}, fetch() {{ return new Response('no', {{ status: 500 }}); }} }})"
    );

    let request = request(temp_dir.path(), &script, port, duration_workload("/", 1, 2));

    let metrics = runtime::run_with_config(
        &temp_dir.path().canonicalize().unwrap(),
        None,
        &request,
        &test_env(),
        &fast_config(),
    )
    .unwrap();

    assert_eq!(metrics.startup_time.status, "unhealthy");
    assert!(metrics.cpu.is_none());

    assert_port_released(port);
}

#[test]
fn target_exit_during_the_workload_keeps_partial_cpu() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let request = request(
        temp_dir.path(),
        &dying_script(port, 1500),
        port,
        duration_workload("/", 30, 4),
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

    assert_eq!(latency.status, "target_exited");

    let cpu = metrics.cpu.unwrap();

    assert!(cpu.average_percent >= 0.0);
    assert!(cpu.peak_percent >= cpu.average_percent);

    assert_port_released(port);
}
