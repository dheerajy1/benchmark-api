#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use benchmark_api::features::v1::benchmark::{
    docker::{Docker, RunningContainer},
    live::{RunContext, RunControl, sink::NoopSink},
    metrics::memory::MemoryConfig,
    model::{BenchmarkRequest, BuildCommand},
    stop::Stop,
    workspace::{
        container::EnvFile,
        orchestrator::{precheck, run_all},
        result::WorkspaceResult,
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn fast() -> MemoryConfig {
    MemoryConfig {
        stabilization: Duration::from_millis(50),
        sample_interval: Duration::from_millis(20),
    }
}

// What the fake Docker answers, and what it was asked
struct FakeDocker {
    running: Vec<RunningContainer>,
    image_bytes: u64,
    // The process that stands in for the container's main process
    pid: u32,
    stopped: Mutex<Vec<String>>,
}

impl Docker for FakeDocker {
    fn running_containers(&self) -> Result<Vec<RunningContainer>, String> {
        Ok(self.running.clone())
    }

    fn image_size_bytes(&self, _image: &str) -> Result<u64, String> {
        Ok(self.image_bytes)
    }

    fn container_pid(&self, _container: &str) -> Result<Option<u32>, String> {
        Ok(Some(self.pid))
    }

    fn stop_and_remove(&self, container: &str) -> Result<(), String> {
        self.stopped.lock().unwrap().push(container.to_string());

        Ok(())
    }

    // A command that prints the ready line, instead of `docker logs`
    fn logs_command(&self, _container: &str) -> BuildCommand {
        BuildCommand {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "echo container-ready; sleep 30".to_string(),
            ],
        }
    }
}

fn standing_in_container() -> Child {
    Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

fn fake(pid: u32) -> FakeDocker {
    FakeDocker {
        running: Vec::new(),
        image_bytes: 4096,
        pid,
        stopped: Mutex::new(Vec::new()),
    }
}

fn request(directory: &Path, workspaces: Value) -> BenchmarkRequest {
    serde_json::from_value(json!({ "path": directory, "workspaces": workspaces })).unwrap()
}

fn project() -> TempDir {
    let dir = TempDir::new().unwrap();

    fs::create_dir(dir.path().join("sub")).unwrap();

    dir
}

fn run(dir: &Path, workspaces: Value, docker: &FakeDocker) -> Result<Vec<WorkspaceResult>, Stop> {
    let request = request(dir, workspaces);

    run_all(
        dir,
        &request.workspaces,
        docker,
        &fast(),
        &RunContext::noop(),
    )
}

fn native(command: &str, ready: &str) -> Value {
    json!({
        "name": "subscriber",
        "directory": "sub",
        "ready_log_line": ready,
        "ready_timeout_seconds": 3,
        "idle_seconds": 1,
        "native": { "enabled": true, "command": command }
    })
}

#[test]
fn native_waits_for_the_ready_line_then_measures_the_idle_window() {
    let dir = project();
    let results = run(
        dir.path(),
        json!([native(
            "sh -c 'echo subscriber-ready; sleep 30'",
            "subscriber-ready"
        )]),
        &fake(0),
    )
    .unwrap();

    let result = &results[0];

    assert_eq!(result.mode, "native");
    assert_eq!(result.status, "success", "{:?}", result.message);
    assert!(result.startup.is_some());
    assert!(result.memory.as_ref().unwrap().baseline.rss_bytes > 0);
    assert!(result.cpu.is_some());
    assert_eq!(result.idle_seconds, Some(1));
    assert!(result.load.is_none());
    assert!(result.image.is_none());
}

#[test]
fn native_runs_the_load_command_while_sampling() {
    let dir = project();
    let mut workspace = native("sh -c 'echo up; sleep 30'", "up");
    workspace["load"] = json!({ "command": "sleep 1" });

    let results = run(dir.path(), json!([workspace]), &fake(0)).unwrap();
    let result = &results[0];

    assert_eq!(result.status, "success", "{:?}", result.message);

    let load = result.load.as_ref().unwrap();

    assert_eq!(load.status, "success");
    assert_eq!(load.exit_code, Some(0));
    assert!(load.duration_ms >= 900);
    // No idle window when a load command ran
    assert_eq!(result.idle_seconds, None);
}

#[test]
fn a_failing_load_command_is_recorded_and_fails_the_mode() {
    let dir = project();
    let mut workspace = native("sh -c 'echo up; sleep 30'", "up");
    workspace["load"] = json!({ "command": "sh -c 'exit 3'" });

    let results = run(dir.path(), json!([workspace]), &fake(0)).unwrap();
    let result = &results[0];

    assert_eq!(result.status, "failed");
    assert_eq!(result.load.as_ref().unwrap().exit_code, Some(3));
}

#[test]
fn a_ready_line_that_never_appears_times_out_with_null_numbers() {
    let dir = project();
    let mut workspace = native("sh -c 'echo nothing-useful; sleep 30'", "never-printed");
    workspace["ready_timeout_seconds"] = json!(1);

    let results = run(dir.path(), json!([workspace]), &fake(0)).unwrap();
    let result = &results[0];

    assert_eq!(result.status, "timeout");
    assert!(result.startup.is_none());
    assert!(result.memory.is_none());
    assert!(result.cpu.is_none());
}

#[test]
fn a_process_that_exits_before_ready_fails() {
    let dir = project();
    let results = run(
        dir.path(),
        json!([native("sh -c 'exit 7'", "never-printed")]),
        &fake(0),
    )
    .unwrap();

    assert_eq!(results[0].status, "failed");
    assert!(results[0].message.as_ref().unwrap().contains("code 7"));
}

#[test]
fn native_receives_the_env_and_results_keep_only_names() {
    let dir = project();
    let mut workspace = native(
        "sh -c 'echo token-is-$SUB_TOKEN; sleep 30'",
        "token-is-canary-value",
    );
    workspace["env"] = json!({ "pasted": "# comment\nexport SUB_TOKEN=canary-value\nOTHER=1\n" });

    let results = run(dir.path(), json!([workspace]), &fake(0)).unwrap();
    let result = &results[0];

    assert_eq!(result.status, "success", "{:?}", result.message);
    assert_eq!(result.env_variable_names, vec!["OTHER", "SUB_TOKEN"]);

    let serialized = serde_json::to_string(result).unwrap();

    assert!(!serialized.contains("canary-value"));
}

#[test]
fn native_can_read_its_env_from_a_file() {
    let dir = project();

    fs::write(dir.path().join("sub/.env"), "SUB_TOKEN=from-file\r\n").unwrap();

    let mut workspace = native(
        "sh -c 'echo token-is-$SUB_TOKEN; sleep 30'",
        "token-is-from-file",
    );
    workspace["env"] = json!({ "file_path": "sub/.env" });

    let results = run(dir.path(), json!([workspace]), &fake(0)).unwrap();

    assert_eq!(results[0].status, "success", "{:?}", results[0].message);
}

#[test]
fn a_missing_env_file_fails_every_enabled_mode_without_starting_anything() {
    let dir = project();
    let mut workspace = native("sh -c 'echo up; sleep 30'", "up");
    workspace["env"] = json!({ "file_path": "missing.env" });

    let results = run(dir.path(), json!([workspace]), &fake(0)).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "failed");
    assert!(results[0].message.as_ref().unwrap().contains("missing.env"));
}

#[test]
fn a_native_build_failure_is_recorded_and_stops_that_mode() {
    let dir = project();
    let mut workspace = native("sh -c 'echo up; sleep 30'", "up");
    workspace["native"]["build_command"] = json!("sh -c 'exit 2'");

    let results = run(dir.path(), json!([workspace]), &fake(0)).unwrap();
    let result = &results[0];

    assert_eq!(result.status, "failed");
    assert_eq!(result.build.as_ref().unwrap().status, "failed");
    assert_eq!(result.build.as_ref().unwrap().exit_code, Some(2));
    assert!(result.startup.is_none());
}

fn docker_workspace(run_command: &str) -> Value {
    json!({
        "name": "subscriber",
        "directory": "sub",
        "ready_log_line": "container-ready",
        "ready_timeout_seconds": 5,
        "idle_seconds": 1,
        "docker": {
            "enabled": true,
            "build_command": "true",
            "image": "recordlog-sub:latest",
            "container_name": "recordlog-sub",
            "run_command": run_command
        }
    })
}

#[test]
fn docker_records_image_size_build_startup_and_memory_of_the_container_process() {
    let dir = project();
    let mut standing = standing_in_container();
    let docker = fake(standing.id());

    let results = run(dir.path(), json!([docker_workspace("true")]), &docker).unwrap();
    let result = &results[0];

    let _ = standing.kill();
    let _ = standing.wait();

    assert_eq!(result.mode, "docker");
    assert_eq!(result.status, "success", "{:?}", result.message);
    assert_eq!(result.image.as_ref().unwrap().bytes, 4096);
    assert_eq!(result.build.as_ref().unwrap().status, "success");
    assert!(result.startup.is_some());
    assert!(result.memory.as_ref().unwrap().baseline.rss_bytes > 0);
    assert!(result.cpu.is_some());

    // The container is removed once before the start and once after the measurement
    assert!(docker.stopped.lock().unwrap().len() >= 2);
}

#[test]
fn docker_build_failure_keeps_its_status_and_skips_the_container() {
    let dir = project();
    let docker = fake(1);
    let mut workspace = docker_workspace("true");
    workspace["docker"]["build_command"] = json!("sh -c 'exit 5'");

    let results = run(dir.path(), json!([workspace]), &docker).unwrap();
    let result = &results[0];

    assert_eq!(result.status, "failed");
    assert_eq!(result.build.as_ref().unwrap().exit_code, Some(5));
    assert!(result.image.is_none());
    assert!(result.memory.is_none());
    assert!(docker.stopped.lock().unwrap().is_empty());
}

#[test]
fn a_failing_docker_mode_does_not_stop_the_native_mode() {
    let dir = project();
    let mut workspace = native("sh -c 'echo up; sleep 30'", "up");
    let mut docker_part = docker_workspace("true");
    docker_part["docker"]["build_command"] = json!("false");
    workspace["docker"] = docker_part["docker"].clone();

    let results = run(dir.path(), json!([workspace]), &fake(1)).unwrap();

    // Native first, Docker second
    assert_eq!(results[0].mode, "native");
    assert_eq!(results[0].status, "success", "{:?}", results[0].message);
    assert_eq!(results[1].mode, "docker");
    assert_eq!(results[1].status, "failed");
}

#[test]
fn docker_gets_the_variables_only_through_an_owner_only_file() {
    let dir = project();
    let captured = dir.path().join("captured.env");
    let mut standing = standing_in_container();
    let docker = fake(standing.id());

    let mut workspace = docker_workspace(&format!("cp {{ENV_FILE}} {}", captured.display()));
    workspace["env"] = json!({ "pasted": "SUB_TOKEN=canary-value\nOTHER=1\n" });

    let results = run(dir.path(), json!([workspace]), &docker).unwrap();

    let _ = standing.kill();
    let _ = standing.wait();

    assert_eq!(results[0].status, "success", "{:?}", results[0].message);
    assert_eq!(
        fs::read_to_string(&captured).unwrap(),
        "OTHER=1\nSUB_TOKEN=canary-value\n"
    );
    // cp gives the copy the source's mode, so the file Docker read was owner-only
    assert_eq!(
        fs::metadata(&captured).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        !serde_json::to_string(&results[0])
            .unwrap()
            .contains("canary-value")
    );
}

#[test]
fn the_env_file_is_private_and_removed_on_drop() {
    let variables = [("A".to_string(), "1".to_string())].into_iter().collect();
    let file = EnvFile::create(&variables).unwrap();
    let path = file.path().to_path_buf();

    assert_eq!(fs::read_to_string(&path).unwrap(), "A=1\n");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    drop(file);

    assert!(!path.exists());
}

#[test]
fn a_value_with_a_line_break_cannot_go_into_a_docker_env_file() {
    let variables = [("A".to_string(), "x\ny".to_string())]
        .into_iter()
        .collect();
    let error = EnvFile::create(&variables).err().unwrap();

    assert!(error.contains('A'));
    assert!(!error.contains("x\ny"));
}

#[test]
fn a_cancel_request_stops_the_run_and_leaves_nothing_running() {
    let dir = project();
    let request = request(
        dir.path(),
        json!([native("sh -c 'echo up; sleep 30'", "up")]),
    );
    let control = RunControl::default();

    let _ = control.request_cancel();

    let ctx = RunContext::with_control(Arc::new(NoopSink), control);
    let outcome = run_all(dir.path(), &request.workspaces, &fake(0), &fast(), &ctx);

    assert_eq!(outcome.err(), Some(Stop::Cancelled));
}

#[test]
fn precheck_refuses_a_running_container_by_name_or_image() {
    let dir = project();
    let request = request(dir.path(), json!([docker_workspace("true")]));

    let mut docker = fake(1);
    docker.running = vec![RunningContainer {
        name: "recordlog-sub".to_string(),
        image: "other:1".to_string(),
    }];

    let message = precheck(dir.path(), &request.workspaces, &docker).unwrap_err();
    assert!(message.contains("docker stop recordlog-sub"));

    docker.running = vec![RunningContainer {
        name: "someone-else".to_string(),
        image: "recordlog-sub".to_string(),
    }];
    assert!(precheck(dir.path(), &request.workspaces, &docker).is_err());

    docker.running = vec![RunningContainer {
        name: "unrelated".to_string(),
        image: "nginx:1".to_string(),
    }];
    assert!(precheck(dir.path(), &request.workspaces, &docker).is_ok());
}

#[test]
fn precheck_refuses_a_running_native_process_in_the_workspace_folder_only() {
    let dir = project();
    // An unusual argument so nothing else on the machine matches
    let request = request(dir.path(), json!([native("sleep 31337", "up")]));

    assert!(precheck(dir.path(), &request.workspaces, &fake(0)).is_ok());

    let mut elsewhere = Command::new("sleep")
        .arg("31337")
        .current_dir(std::env::temp_dir())
        .spawn()
        .unwrap();

    // Same command, another working directory: not the workspace
    assert!(precheck(dir.path(), &request.workspaces, &fake(0)).is_ok());

    let mut inside = Command::new("sleep")
        .arg("31337")
        .current_dir(dir.path().join("sub"))
        .spawn()
        .unwrap();

    let message = precheck(dir.path(), &request.workspaces, &fake(0)).unwrap_err();

    let _ = elsewhere.kill();
    let _ = elsewhere.wait();
    let _ = inside.kill();
    let _ = inside.wait();

    assert!(message.contains("already running natively"));
}
