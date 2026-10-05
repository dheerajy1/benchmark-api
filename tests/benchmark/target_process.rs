use std::{fs, net::TcpListener, path::Path, thread, time::Duration};

use benchmark_api::config::env::Env;
use benchmark_api::features::v1::benchmark::{
    metrics::memory::{self, MemoryConfig},
    model::BuildCommand,
    target_process::{ReadyOutcome, TargetProcess},
};
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

fn fast_config() -> MemoryConfig {
    MemoryConfig {
        stabilization: Duration::from_millis(100),
        sample_interval: Duration::from_millis(50),
        sample_window: Duration::from_millis(300),
    }
}

fn write_server_project(temp_dir: &TempDir, port: u16) {
    let package_json = r#"{
        "name": "target-process-test",
        "scripts": {
            "start": "bun --env-file=.env -e \"Bun.serve({ port: process.env.APP_PORT, fetch() { return new Response('ok'); } })\""
        }
    }"#;

    fs::write(temp_dir.path().join("package.json"), package_json).unwrap();
    fs::write(temp_dir.path().join(".env"), format!("APP_PORT={port}\n")).unwrap();
}

fn start_command() -> BuildCommand {
    BuildCommand {
        program: "bun".to_string(),
        args: vec!["run".to_string(), "start".to_string()],
    }
}

fn start_ready(dir: &Path, port: u16) -> TargetProcess {
    let mut process = TargetProcess::start(&start_command(), dir).unwrap();

    let outcome = process
        .wait_until_ready(port, &test_env(), Duration::from_secs(5))
        .unwrap();

    assert_eq!(outcome, ReadyOutcome::Ready);

    process
}

#[test]
fn target_stays_alive_after_readiness() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);

    thread::sleep(Duration::from_millis(300));

    assert!(process.is_running());
    assert!(TcpListener::bind(("127.0.0.1", port)).is_err());

    process.shutdown();
}

#[test]
fn target_stays_alive_through_memory_measurement() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);

    let result = memory::measure_with_config(&mut process, &fast_config()).unwrap();

    assert!(result.baseline.rss_bytes > 0);
    assert!(result.peak.rss_bytes >= result.baseline.rss_bytes);
    assert_eq!(
        result.delta.rss_bytes,
        result.peak.rss_bytes - result.baseline.rss_bytes
    );

    assert!(process.is_running());
    assert!(TcpListener::bind(("127.0.0.1", port)).is_err());

    process.shutdown();
}

#[test]
fn group_rss_covers_the_process_group() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);

    let rss = memory::read_group_rss(process.process_group_id()).unwrap();

    assert!(rss > 0);

    process.shutdown();
}

#[test]
fn shutdown_cleans_up_target_and_children() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);
    let group_id = process.process_group_id();

    process.shutdown();

    thread::sleep(Duration::from_millis(200));

    assert!(!process.is_running());
    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
    assert!(memory::read_group_rss(group_id).is_err());
}

#[test]
fn shutdown_can_be_called_twice() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);

    process.shutdown();
    process.shutdown();

    assert!(!process.is_running());
}

#[test]
fn memory_measurement_fails_when_target_is_gone() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);

    process.shutdown();

    let result = memory::measure_with_config(&mut process, &fast_config());

    assert!(result.is_err());
}
