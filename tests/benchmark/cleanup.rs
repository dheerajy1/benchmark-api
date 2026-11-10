use std::fs;

use benchmark_api::config::env::Env;
use benchmark_api::features::v1::benchmark::{
    cleanup,
    model::{BenchmarkRequest, CleanupRequest},
    service, validation,
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

fn request(path: &str, cleanup: Value) -> BenchmarkRequest {
    serde_json::from_value(json!({
        "path": path,
        "build": { "command": "true", "output_dir": "target" },
        "cleanup": cleanup,
    }))
    .unwrap()
}

// A request with a Docker workspace, the docker object is merged over a valid one
fn with_docker(build_cleanup: Value, docker: Value) -> BenchmarkRequest {
    let mut workspace = json!({
        "name": "subscriber",
        "directory": "subscriber",
        "ready_log_line": "subscriber ready",
        "docker": {
            "enabled": true,
            "build_command": "docker build -t recordlog-sub .",
            "image": "recordlog-sub:latest",
            "container_name": "recordlog-sub",
            "run_command": "docker run --name recordlog-sub recordlog-sub:latest"
        }
    });

    for (key, value) in docker.as_object().unwrap() {
        workspace["docker"][key] = value.clone();
    }

    serde_json::from_value(json!({
        "path": "/tmp/project",
        "build": { "command": "true" },
        "cleanup": build_cleanup,
        "workspaces": [workspace],
    }))
    .unwrap()
}

// A project whose build output folder exists, as it does after a real build
fn project() -> TempDir {
    let temp_dir = TempDir::new().unwrap();

    fs::create_dir_all(temp_dir.path().join("target")).unwrap();

    temp_dir
}

fn cleanup_request(command: &str, timeout_seconds: Option<u64>) -> CleanupRequest {
    CleanupRequest {
        command: command.to_string(),
        timeout_seconds,
    }
}

#[test]
fn a_request_without_cleanup_has_no_step() {
    let request: BenchmarkRequest = serde_json::from_value(json!({
        "path": "/tmp/project",
        "build": { "command": "true" },
    }))
    .unwrap();

    assert!(request.cleanup.is_none());
    assert!(cleanup::plan(&request).unwrap().is_empty());
    assert!(!cleanup::is_requested(&request));
}

#[test]
fn an_empty_command_is_ignored() {
    let request = request(
        "/tmp/project",
        json!({ "command": "   ", "timeout_seconds": 0 }),
    );

    assert!(validation::validate(&request).is_ok());
    assert!(cleanup::plan(&request).unwrap().is_empty());
}

#[test]
fn the_default_timeout_is_120_seconds() {
    let request = request(
        "/tmp/project",
        serde_json::to_value(json!({ "command": "docker system prune -f" })).unwrap(),
    );

    let steps = cleanup::plan(&request).unwrap();

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].timeout.as_secs(), 120);
    assert_eq!(steps[0].command.program, "docker");
    assert_eq!(steps[0].command.args, vec!["system", "prune", "-f"]);
    assert_eq!(steps[0].stage_key(), "cleanup:build");
}

#[test]
fn a_given_timeout_is_used() {
    let request = request(
        "/tmp/project",
        json!({ "command": "true", "timeout_seconds": 30 }),
    );

    assert_eq!(cleanup::plan(&request).unwrap()[0].timeout.as_secs(), 30);
}

#[test]
fn rejects_a_timeout_out_of_range() {
    for seconds in [0, 3601] {
        let error = validation::validate(&request(
            "/tmp/project",
            json!({ "command": "true", "timeout_seconds": seconds }),
        ))
        .unwrap_err();

        assert!(error.contains("cleanup.timeout_seconds"), "{error}");

        let error = validation::validate(&with_docker(
            Value::Null,
            json!({ "cleanup": { "command": "true", "timeout_seconds": seconds } }),
        ))
        .unwrap_err();

        assert!(error.contains("docker.cleanup.timeout_seconds"), "{error}");
    }
}

#[test]
fn rejects_a_command_that_cannot_be_split() {
    let error = validation::validate(&request(
        "/tmp/project",
        json!({ "command": "echo \"open" }),
    ))
    .unwrap_err();

    assert!(error.contains("cleanup.command"), "{error}");

    let error = validation::validate(&with_docker(
        Value::Null,
        json!({ "cleanup": { "command": "echo \"open" } }),
    ))
    .unwrap_err();

    assert!(error.contains("docker.cleanup.command"), "{error}");
}

#[test]
fn the_docker_cleanup_follows_the_build_cleanup() {
    let request = with_docker(
        json!({ "command": "true" }),
        json!({ "cleanup": { "command": "docker rmi recordlog-sub:latest", "timeout_seconds": 45 } }),
    );

    assert!(validation::validate(&request).is_ok());

    let steps = cleanup::plan(&request).unwrap();
    let keys: Vec<String> = steps.iter().map(|step| step.stage_key()).collect();

    assert_eq!(keys, vec!["cleanup:build", "cleanup:docker:subscriber"]);
    assert_eq!(steps[1].timeout.as_secs(), 45);
    assert!(cleanup::is_requested(&request));
}

#[test]
fn an_empty_docker_cleanup_or_a_docker_mode_that_is_off_is_skipped() {
    let empty = with_docker(Value::Null, json!({ "cleanup": { "command": "  " } }));

    assert!(cleanup::plan(&empty).unwrap().is_empty());

    let off = with_docker(
        Value::Null,
        json!({ "enabled": false, "cleanup": { "command": "docker rmi x" } }),
    );

    assert!(cleanup::plan(&off).unwrap().is_empty());
}

#[test]
fn the_command_runs_in_the_project_path() {
    let temp_dir = project();
    let path = temp_dir.path().to_str().unwrap();

    let request = request(
        path,
        json!({ "command": "sh -c \"echo cleaned > cleanup_marker\"" }),
    );

    service::run(&request, &test_env()).unwrap();

    assert_eq!(
        fs::read_to_string(temp_dir.path().join("cleanup_marker"))
            .unwrap()
            .trim(),
        "cleaned"
    );
}

#[test]
fn a_failing_cleanup_does_not_fail_a_run_that_worked() {
    let temp_dir = project();
    let path = temp_dir.path().to_str().unwrap();

    let request = request(path, json!({ "command": "sh -c \"exit 3\"" }));

    assert!(service::run(&request, &test_env()).is_ok());

    let step = &cleanup::plan(&request).unwrap()[0];
    let outcome = cleanup::run(temp_dir.path(), step);

    assert_eq!(outcome.status, "failed");
    assert!(outcome.message.unwrap().contains("exit code 3"));
}

#[test]
fn a_cleanup_that_takes_too_long_times_out_and_the_run_still_works() {
    let temp_dir = project();
    let path = temp_dir.path().to_str().unwrap();

    let request = request(
        path,
        json!({ "command": "sh -c \"sleep 30\"", "timeout_seconds": 1 }),
    );

    assert!(service::run(&request, &test_env()).is_ok());

    let step = &cleanup::plan(&request).unwrap()[0];
    let outcome = cleanup::run(temp_dir.path(), step);

    assert_eq!(outcome.status, "timeout");
    assert!(outcome.message.unwrap().contains("1 seconds"));
}

#[test]
fn the_cleanup_runs_even_when_the_build_failed() {
    let temp_dir = project();
    let path = temp_dir.path().to_str().unwrap();

    let request: BenchmarkRequest = serde_json::from_value(json!({
        "path": path,
        "build": { "command": "false", "output_dir": "target" },
        "cleanup": { "command": "sh -c \"touch cleanup_marker\"" },
    }))
    .unwrap();

    let _ = service::run(&request, &test_env());

    assert!(temp_dir.path().join("cleanup_marker").exists());
}

#[test]
fn cleanup_request_builds_from_plain_values() {
    assert!(cleanup::is_set(&cleanup_request("true", None)));
    assert!(!cleanup::is_set(&cleanup_request("  ", Some(5))));
}
