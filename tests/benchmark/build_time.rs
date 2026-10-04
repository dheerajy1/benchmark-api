use std::{fs, time::Duration};

use benchmark_api::features::v1::benchmark::metrics::build_time;
use tempfile::TempDir;

fn write_package_json(temp_dir: &TempDir, scripts: &str) {
    let package_json = format!(
        r#"{{
            "name": "build-time-test",
            "scripts": {}
        }}"#,
        scripts
    );

    fs::write(temp_dir.path().join("package.json"), package_json).unwrap();
}

#[test]
fn discovers_build_command_for_node() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"{"build":"some-node-build-command"}"#);

    let command = build_time::discover_build_command(temp_dir.path(), Some("node"))
        .unwrap()
        .unwrap();

    assert_eq!(command.program, "npm");
    assert_eq!(command.args, ["run", "build"]);
}

#[test]
fn discovers_build_command_for_bun() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"{"build":"some-bun-build-command"}"#);

    let command = build_time::discover_build_command(temp_dir.path(), Some("bun"))
        .unwrap()
        .unwrap();

    assert_eq!(command.program, "bun");
    assert_eq!(command.args, ["run", "build"]);
}

#[test]
fn returns_no_build_command_without_build_script() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"{"test":"bun -e \"process.exit(0)\""}"#);

    let result = build_time::measure(temp_dir.path(), Some("bun")).unwrap();

    assert_eq!(result.status, "no_build_command");
    assert!(result.duration_ms.is_none());
    assert!(result.command.is_none());
    assert!(result.working_directory.is_none());
    assert!(result.started_at.is_none());
    assert!(result.finished_at.is_none());
    assert!(result.exit_code.is_none());
}

#[test]
fn returns_no_build_command_for_unsupported_runtime() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"{"build":"bun -e \"process.exit(0)\""}"#);

    let result = build_time::measure(temp_dir.path(), Some("rust")).unwrap();

    assert_eq!(result.status, "no_build_command");
    assert!(result.command.is_none());
    assert!(result.duration_ms.is_none());
}

#[test]
fn successful_build_reports_success() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"{"build":"bun -e \"process.exit(0)\""}"#);

    let result = build_time::measure(temp_dir.path(), Some("bun")).unwrap();

    assert_eq!(result.status, "success");
    assert_eq!(result.exit_code, Some(0));
    assert!(result.duration_ms.is_some());
}

#[test]
fn failed_build_reports_failure_and_exit_code() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"{"build":"bun -e \"process.exit(7)\""}"#);

    let result = build_time::measure(temp_dir.path(), Some("bun")).unwrap();

    assert_eq!(result.status, "failed");
    assert_eq!(result.exit_code, Some(7));
    assert!(result.duration_ms.is_some());
}

#[test]
fn timeout_reports_timeout_and_no_exit_code() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(
        &temp_dir,
        r#"{"build":"bun -e \"setTimeout(() => {}, 10000)\""}"#,
    );

    let result =
        build_time::measure_with_timeout(temp_dir.path(), Some("bun"), Duration::from_millis(100))
            .unwrap();

    assert_eq!(result.status, "timeout");
    assert!(result.duration_ms.is_some());
    assert!(result.exit_code.is_none());
}

#[test]
fn duration_is_measured_with_elapsed_time() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(
        &temp_dir,
        r#"{"build":"bun -e \"setTimeout(() => {}, 150)\""}"#,
    );

    let result = build_time::measure(temp_dir.path(), Some("bun")).unwrap();

    assert_eq!(result.status, "success");

    let duration_ms = result.duration_ms.unwrap();

    assert!(duration_ms >= 100);
    assert!(duration_ms < 5_000);
}

#[test]
fn command_and_working_directory_are_correct() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"{"build":"bun -e \"process.exit(0)\""}"#);

    let result = build_time::measure(temp_dir.path(), Some("bun")).unwrap();

    let command = result.command.unwrap();

    assert_eq!(command.program, "bun");
    assert_eq!(command.args, ["run", "build"]);

    assert_eq!(
        result.working_directory.unwrap(),
        temp_dir
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
}

#[test]
fn timestamps_are_present_for_executed_build() {
    let temp_dir = TempDir::new().unwrap();

    write_package_json(&temp_dir, r#"{"build":"bun -e \"process.exit(0)\""}"#);

    let result = build_time::measure(temp_dir.path(), Some("bun")).unwrap();

    assert!(result.started_at.is_some());
    assert!(result.finished_at.is_some());

    assert!(result.started_at.as_ref().unwrap().ends_with('Z'));
    assert!(result.finished_at.as_ref().unwrap().ends_with('Z'));
}
