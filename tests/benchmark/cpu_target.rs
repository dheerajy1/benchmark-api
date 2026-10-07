use std::{fs, net::TcpListener, path::Path, thread, time::Duration};

use benchmark_api::config::env::Env;
use benchmark_api::features::v1::benchmark::{
    metrics::cpu::{baseline, proc::read_group_ticks, session},
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

fn write_server_project(temp_dir: &TempDir, port: u16) {
    let package_json = r#"{
        "name": "cpu-target-test",
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
fn group_ticks_cover_the_target_and_exclude_benchmark_api() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);
    let group_id = process.process_group_id();

    let readings = read_group_ticks(group_id).unwrap();

    assert!(readings.iter().any(|item| item.pid == group_id));
    assert!(readings.iter().all(|item| item.pid != std::process::id()));

    process.shutdown();
}

#[test]
fn baseline_is_measured_for_a_live_target() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);

    let percent = baseline::measure_baseline_for_group(
        Duration::from_millis(100),
        process.process_group_id(),
    )
    .unwrap();

    assert!(percent >= 0.0);
    assert!(process.is_running());

    process.shutdown();
}

#[test]
fn cpu_observer_never_terminates_the_target() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);

    let started =
        session::begin_for_group(0.0, Duration::from_millis(50), process.process_group_id())
            .unwrap();

    thread::sleep(Duration::from_millis(300));

    let outcome = started.finish().unwrap();
    let cpu = outcome.cpu.unwrap();

    assert!(!outcome.target_gone);
    assert!(cpu.average_percent >= 0.0);
    assert!(cpu.peak_percent >= cpu.average_percent);

    assert!(process.is_running());
    assert!(TcpListener::bind(("127.0.0.1", port)).is_err());

    process.shutdown();
}

#[test]
fn shutdown_leaves_no_process_group_to_measure() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);
    let group_id = process.process_group_id();

    process.shutdown();

    thread::sleep(Duration::from_millis(200));

    assert!(!process.is_running());
    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
    assert!(read_group_ticks(group_id).is_err());
    assert!(baseline::measure_baseline_for_group(Duration::from_millis(10), group_id).is_err());
    assert!(session::begin_for_group(0.0, Duration::from_millis(50), group_id).is_err());
}

#[test]
fn target_disappearing_during_sampling_is_reported() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_server_project(&temp_dir, port);

    let mut process = start_ready(&temp_dir.path().canonicalize().unwrap(), port);

    let started =
        session::begin_for_group(0.0, Duration::from_millis(50), process.process_group_id())
            .unwrap();

    thread::sleep(Duration::from_millis(150));

    process.shutdown();

    thread::sleep(Duration::from_millis(300));

    let outcome = started.finish().unwrap();

    assert!(outcome.target_gone);
}
