use std::time::Duration;

use benchmark_api::features::v1::benchmark::{metrics::build_time, model::BuildCommand};
use tempfile::TempDir;

// A build that runs a small bun script, the way a person would write `bun -e "..."` as their build
fn bun(script: &str) -> BuildCommand {
    BuildCommand {
        program: "bun".to_string(),
        args: vec!["-e".to_string(), script.to_string()],
    }
}

#[test]
fn successful_build_reports_success() {
    let temp_dir = TempDir::new().unwrap();

    let result = build_time::measure(temp_dir.path(), &bun("process.exit(0)")).unwrap();

    assert_eq!(result.status, "success");
    assert_eq!(result.exit_code, Some(0));
    assert!(result.duration_ms.is_some());
}

#[test]
fn failed_build_reports_failure_and_exit_code() {
    let temp_dir = TempDir::new().unwrap();

    let result = build_time::measure(temp_dir.path(), &bun("process.exit(7)")).unwrap();

    assert_eq!(result.status, "failed");
    assert_eq!(result.exit_code, Some(7));
    assert!(result.duration_ms.is_some());
}

#[test]
fn timeout_reports_timeout_and_no_exit_code() {
    let temp_dir = TempDir::new().unwrap();

    let result = build_time::measure_with_timeout(
        temp_dir.path(),
        &bun("setTimeout(() => {}, 10000)"),
        Duration::from_millis(100),
    )
    .unwrap();

    assert_eq!(result.status, "timeout");
    assert!(result.duration_ms.is_some());
    assert!(result.exit_code.is_none());
}

#[test]
fn duration_is_measured_with_elapsed_time() {
    let temp_dir = TempDir::new().unwrap();

    let result = build_time::measure(temp_dir.path(), &bun("setTimeout(() => {}, 150)")).unwrap();

    assert_eq!(result.status, "success");

    let duration_ms = result.duration_ms.unwrap();

    assert!(duration_ms >= 100);
    assert!(duration_ms < 5_000);
}

#[test]
fn command_and_working_directory_are_correct() {
    let temp_dir = TempDir::new().unwrap();
    let command = bun("process.exit(0)");

    let result = build_time::measure(temp_dir.path(), &command).unwrap();

    assert_eq!(result.command.unwrap(), command);

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

    let result = build_time::measure(temp_dir.path(), &bun("process.exit(0)")).unwrap();

    assert!(result.started_at.is_some());
    assert!(result.finished_at.is_some());

    assert!(result.started_at.as_ref().unwrap().ends_with('Z'));
    assert!(result.finished_at.as_ref().unwrap().ends_with('Z'));
}

#[test]
fn a_program_that_does_not_exist_is_a_failed_build() {
    let temp_dir = TempDir::new().unwrap();
    let command = BuildCommand {
        program: "definitely-not-a-real-program".to_string(),
        args: Vec::new(),
    };

    let result = build_time::measure(temp_dir.path(), &command).unwrap();

    assert_eq!(result.status, "failed");
    assert!(result.exit_code.is_some());
}
