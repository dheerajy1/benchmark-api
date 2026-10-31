use benchmark_api::features::v1::benchmark::{model::BenchmarkRequest, validation};
use serde_json::{Value, json};

fn native_only() -> Value {
    json!({
        "name": "subscriber",
        "directory": "subscriber",
        "ready_log_line": "subscriber ready",
        "native": { "enabled": true, "command": "bun run src/index.ts" }
    })
}

fn docker_only() -> Value {
    json!({
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
    })
}

fn request(workspaces: Value) -> BenchmarkRequest {
    serde_json::from_value(json!({ "path": "/tmp/project", "workspaces": workspaces })).unwrap()
}

fn error_of(workspaces: Value) -> String {
    validation::validate(&request(workspaces)).unwrap_err()
}

// The workspace with one field replaced, set to null removes it
fn with(mut workspace: Value, field: &str, value: Value) -> Value {
    if value.is_null() {
        if let Some(object) = workspace.as_object_mut() {
            object.remove(field);
        }
    } else {
        workspace[field] = value;
    }

    workspace
}

#[test]
fn a_request_without_workspaces_is_unchanged() {
    let request: BenchmarkRequest =
        serde_json::from_value(json!({ "path": "/tmp/project" })).unwrap();

    assert!(request.workspaces.is_empty());
    assert!(validation::validate(&request).is_ok());
}

#[test]
fn accepts_native_only_docker_only_and_both() {
    assert!(validation::validate(&request(json!([native_only()]))).is_ok());
    assert!(validation::validate(&request(json!([docker_only()]))).is_ok());

    let mut both = native_only();
    both["docker"] = docker_only()["docker"].clone();

    assert!(validation::validate(&request(json!([both]))).is_ok());
}

#[test]
fn applies_the_agreed_defaults() {
    let request = request(json!([docker_only()]));
    let workspace = &request.workspaces[0];

    assert_eq!(workspace.idle_seconds, 30);
    assert_eq!(workspace.ready_timeout_seconds, 60);
    assert_eq!(
        workspace.docker.as_ref().unwrap().build_timeout_seconds,
        1800
    );
}

#[test]
fn a_workspace_needs_native_or_docker_enabled() {
    let none = json!({
        "name": "subscriber", "directory": "s", "ready_log_line": "ready"
    });
    assert!(error_of(json!([none])).contains("enable native or docker"));

    let mut disabled = native_only();
    disabled["native"]["enabled"] = json!(false);
    assert!(error_of(json!([disabled])).contains("enable native or docker"));
}

#[test]
fn the_ready_log_line_is_required() {
    let missing = with(native_only(), "ready_log_line", json!(null));
    assert!(error_of(json!([missing])).contains("ready_log_line"));

    let blank = with(native_only(), "ready_log_line", json!("   "));
    assert!(error_of(json!([blank])).contains("ready_log_line"));
}

#[test]
fn env_may_come_from_a_file_or_a_paste_but_not_both() {
    let file = with(native_only(), "env", json!({ "file_path": "/tmp/.env" }));
    assert!(validation::validate(&request(json!([file]))).is_ok());

    let pasted = with(native_only(), "env", json!({ "pasted": "A=1\nB=2\n" }));
    assert!(validation::validate(&request(json!([pasted]))).is_ok());

    let both = with(
        native_only(),
        "env",
        json!({ "file_path": "/tmp/.env", "pasted": "A=1" }),
    );
    assert!(error_of(json!([both])).contains("not both"));

    let neither = with(native_only(), "env", json!({}));
    assert!(validation::validate(&request(json!([neither]))).is_ok());
}

#[test]
fn a_broken_paste_is_reported_without_its_values() {
    let broken = with(
        native_only(),
        "env",
        json!({ "pasted": "TOKEN=\"secret-value-123\nOTHER=1" }),
    );
    let message = error_of(json!([broken]));

    assert!(message.contains("env.pasted"));
    assert!(!message.contains("secret-value-123"));
}

#[test]
fn docker_run_command_needs_the_placeholder_when_env_is_given() {
    let without = with(docker_only(), "env", json!({ "pasted": "A=1" }));
    assert!(error_of(json!([without])).contains("{ENV_FILE}"));

    let mut with_placeholder = with(docker_only(), "env", json!({ "pasted": "A=1" }));
    with_placeholder["docker"]["run_command"] =
        json!("docker run --env-file {ENV_FILE} --name recordlog-sub recordlog-sub:latest");
    assert!(validation::validate(&request(json!([with_placeholder]))).is_ok());
}

#[test]
fn the_placeholder_without_an_env_is_refused() {
    let mut workspace = docker_only();
    workspace["docker"]["run_command"] = json!("docker run --env-file {ENV_FILE} x");

    assert!(error_of(json!([workspace])).contains("no env"));
}

#[test]
fn the_directory_stays_inside_the_project() {
    for directory in ["/etc", "../outside", "a/../../b", ""] {
        let workspace = with(native_only(), "directory", json!(directory));

        assert!(
            error_of(json!([workspace])).contains("directory"),
            "{directory}"
        );
    }
}

#[test]
fn names_must_be_safe_and_unique() {
    let bad = with(native_only(), "name", json!("a b"));
    assert!(error_of(json!([bad])).contains("name"));

    let message = error_of(json!([native_only(), native_only()]));
    assert!(message.contains("used twice"));
}

#[test]
fn docker_names_cannot_look_like_options() {
    let mut workspace = docker_only();
    workspace["docker"]["container_name"] = json!("--privileged");
    assert!(error_of(json!([workspace])).contains("container_name"));

    let mut workspace = docker_only();
    workspace["docker"]["image"] = json!("-v x");
    assert!(error_of(json!([workspace])).contains("image"));
}

#[test]
fn commands_must_be_well_formed() {
    let mut workspace = native_only();
    workspace["native"]["command"] = json!("bun 'unterminated");
    assert!(error_of(json!([workspace])).contains("native.command"));

    let load = with(native_only(), "load", json!({ "command": "  " }));
    assert!(error_of(json!([load])).contains("load.command"));
}

#[test]
fn timeouts_and_windows_must_be_in_range() {
    let zero = with(native_only(), "idle_seconds", json!(0));
    assert!(error_of(json!([zero])).contains("idle_seconds"));

    let zero = with(native_only(), "ready_timeout_seconds", json!(0));
    assert!(error_of(json!([zero])).contains("ready_timeout_seconds"));
}

#[test]
fn unknown_workspace_fields_are_rejected() {
    let result = serde_json::from_value::<BenchmarkRequest>(json!({
        "path": "/tmp/project",
        "workspaces": [with(native_only(), "ready_log_lin", json!("x"))]
    }));

    assert!(result.is_err());
}

#[test]
fn debug_output_never_shows_pasted_values() {
    let pasted = with(
        native_only(),
        "env",
        json!({ "pasted": "TOKEN=hunter2-canary" }),
    );
    let request = request(json!([pasted]));

    assert!(!format!("{request:?}").contains("hunter2-canary"));
}
