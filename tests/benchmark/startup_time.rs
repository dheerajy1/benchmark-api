use std::{fs, net::TcpListener, thread, time::Duration};

use benchmark_api::config::env::Env;
use benchmark_api::features::v1::benchmark::metrics::startup_time;
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

fn write_package_json(temp_dir: &TempDir, start_script: &str) {
    let package_json = format!(
        r#"{{
            "name": "startup-time-test",
            "scripts": {{
                "start": "{}"
            }}
        }}"#,
        start_script.replace('"', "\\\"")
    );

    fs::write(temp_dir.path().join("package.json"), package_json).unwrap();
}

fn write_env(temp_dir: &TempDir, port: u16) {
    fs::write(temp_dir.path().join(".env"), format!("APP_PORT={port}\n")).unwrap();
}

#[test]
fn discovers_bun_start_command() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"bun -e "process.exit(0)""#);

    let command = startup_time::discover_start_command(temp_dir.path(), Some("bun"))
        .unwrap()
        .unwrap();

    assert_eq!(command.program, "bun");
    assert_eq!(command.args, ["run", "start"]);
}

#[test]
fn discovers_node_start_command() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"node server.js"#);

    let command = startup_time::discover_start_command(temp_dir.path(), Some("node"))
        .unwrap()
        .unwrap();

    assert_eq!(command.program, "npm");
    assert_eq!(command.args, ["run", "start"]);
}

#[test]
fn discovers_rust_start_command() {
    let temp_dir = TempDir::new().unwrap();

    let command = startup_time::discover_start_command(temp_dir.path(), Some("rust"))
        .unwrap()
        .unwrap();

    assert_eq!(command.program, "cargo");
    assert_eq!(command.args, ["run"]);
}

#[test]
fn returns_no_start_command_without_start_script() {
    let temp_dir = TempDir::new().unwrap();

    fs::write(
        temp_dir.path().join("package.json"),
        r#"{
            "name": "startup-time-test",
            "scripts": {
                "test": "bun -e \"process.exit(0)\""
            }
        }"#,
    )
    .unwrap();

    let result = startup_time::measure(temp_dir.path(), Some("bun"), &test_env()).unwrap();

    assert_eq!(result.status, "no_start_command");
    assert!(result.duration_ms.is_none());
    assert!(result.command.is_none());
    assert!(result.working_directory.is_none());
    assert!(result.readiness.is_none());
    assert!(result.started_at.is_none());
    assert!(result.ready_at.is_none());
    assert!(result.exit_code.is_none());
}

#[test]
fn returns_no_port_when_port_cannot_be_discovered() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"bun -e "process.exit(0)""#);

    let result = startup_time::measure(temp_dir.path(), Some("bun"), &test_env()).unwrap();

    assert_eq!(result.status, "no_port");
    assert!(result.duration_ms.is_none());
    assert!(result.command.is_some());
    assert!(result.working_directory.is_none());
    assert!(result.readiness.is_none());
    assert!(result.started_at.is_none());
    assert!(result.ready_at.is_none());
    assert!(result.exit_code.is_none());
}

#[test]
fn failed_process_reports_failure() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_package_json(&temp_dir, r#"bun -e "process.exit(7)""#);
    write_env(&temp_dir, port);

    let result = startup_time::measure(temp_dir.path(), Some("bun"), &test_env()).unwrap();

    assert_eq!(result.status, "failed");
    assert_eq!(result.exit_code, Some(7));
    assert!(result.duration_ms.is_some());
    assert!(result.command.is_some());
    assert!(result.readiness.is_some());
    assert!(result.started_at.is_some());
    assert!(result.ready_at.is_none());
}

#[test]
fn timeout_reports_timeout() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_package_json(&temp_dir, r#"bun -e "setTimeout(() => {}, 10000)""#);
    write_env(&temp_dir, port);

    let result = startup_time::measure_with_timeout(
        temp_dir.path(),
        Some("bun"),
        &test_env(),
        Duration::from_millis(150),
    )
    .unwrap();

    assert_eq!(result.status, "timeout");
    assert!(result.duration_ms.is_some());
    assert!(result.command.is_some());
    assert!(result.readiness.is_some());
    assert!(result.started_at.is_some());
    assert!(result.ready_at.is_none());
    assert!(result.exit_code.is_none());
}

#[test]
fn successful_http_readiness_reports_startup_time() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_package_json(
        &temp_dir,
        &format!(
            r#"bun --env-file=.env -e "Bun.serve({{ port: process.env.APP_PORT, fetch() {{ return new Response('ok'); }} }})""#
        ),
    );
    write_env(&temp_dir, port);

    let result = startup_time::measure_with_timeout(
        temp_dir.path(),
        Some("bun"),
        &test_env(),
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(result.status, "success");
    assert!(result.duration_ms.is_some());
    assert!(result.duration_ms.unwrap() < 5_000);

    let command = result.command.unwrap();

    assert_eq!(command.program, "bun");
    assert_eq!(command.args, ["run", "start"]);

    assert_eq!(
        result.working_directory.unwrap(),
        temp_dir
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );

    let readiness = result.readiness.unwrap();

    assert_eq!(readiness.readiness_type, "http");
    assert_eq!(readiness.host, "127.0.0.1");
    assert_eq!(readiness.port, port);
    assert_eq!(readiness.path, "/api/v1/health/app");
    assert_eq!(readiness.method, "GET");
    assert_eq!(readiness.expected_status, 200);

    assert!(result.started_at.is_some());
    assert!(result.ready_at.is_some());
    assert!(result.exit_code.is_none());
}

#[test]
fn readiness_waits_until_http_server_is_available() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_package_json(
        &temp_dir,
        &format!(
            r#"bun --env-file=.env -e "setTimeout(() => Bun.serve({{ port: process.env.APP_PORT, fetch() {{ return new Response('ok'); }} }}), 200)""#
        ),
    );
    write_env(&temp_dir, port);

    let result = startup_time::measure_with_timeout(
        temp_dir.path(),
        Some("bun"),
        &test_env(),
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(result.status, "success");

    let duration_ms = result.duration_ms.unwrap();

    assert!(duration_ms >= 100);
    assert!(duration_ms < 5_000);
}

#[test]
fn startup_process_is_cleaned_up_after_success() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_package_json(
        &temp_dir,
        &format!(
            r#"bun --env-file=.env -e "Bun.serve({{ port: process.env.APP_PORT, fetch() {{ return new Response('ok'); }} }})""#
        ),
    );
    write_env(&temp_dir, port);

    let result = startup_time::measure_with_timeout(
        temp_dir.path(),
        Some("bun"),
        &test_env(),
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(result.status, "success");

    thread::sleep(Duration::from_millis(200));

    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
}

#[test]
fn timestamps_are_present_for_successful_startup() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_package_json(
        &temp_dir,
        &format!(
            r#"bun --env-file=.env -e "Bun.serve({{ port: process.env.APP_PORT, fetch() {{ return new Response('ok'); }} }})""#
        ),
    );
    write_env(&temp_dir, port);

    let result = startup_time::measure_with_timeout(
        temp_dir.path(),
        Some("bun"),
        &test_env(),
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(result.status, "success");
    assert!(result.started_at.as_ref().unwrap().ends_with('Z'));
    assert!(result.ready_at.as_ref().unwrap().ends_with('Z'));
}

#[test]
fn non_200_response_reports_unhealthy() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    write_package_json(
        &temp_dir,
        &format!(
            r#"bun --env-file=.env -e "Bun.serve({{ port: process.env.APP_PORT, fetch() {{ return new Response('no', {{ status: 500 }}); }} }})""#
        ),
    );
    write_env(&temp_dir, port);

    let result = startup_time::measure_with_timeout(
        temp_dir.path(),
        Some("bun"),
        &test_env(),
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(result.status, "unhealthy");
    assert!(result.duration_ms.is_some());
    assert!(result.duration_ms.unwrap() < 5_000);
    assert!(result.readiness.is_some());
    assert!(result.started_at.is_some());
    assert!(result.ready_at.is_none());
    assert!(result.exit_code.is_none());
}

#[test]
fn readiness_request_sends_client_headers() {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    // Server returns 200 only when both headers match test_env, otherwise 403
    write_package_json(
        &temp_dir,
        &format!(
            r#"bun --env-file=.env -e "Bun.serve({{ port: process.env.APP_PORT, fetch(req) {{ return req.headers.get('x-client-id') === 'test-client-id' && req.headers.get('x-client-secret') === 'test-client-secret' ? new Response('ok') : new Response('no', {{ status: 403 }}); }} }})""#
        ),
    );
    write_env(&temp_dir, port);

    let result = startup_time::measure_with_timeout(
        temp_dir.path(),
        Some("bun"),
        &test_env(),
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(result.status, "success");
    assert!(result.ready_at.is_some());
}