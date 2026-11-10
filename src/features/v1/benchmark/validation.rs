use std::path::{Component, Path};

use super::{
    cleanup,
    host::is_loopback,
    model::{BenchmarkRequest, BuildRequest},
    request::StartupRequest,
    workload::{
        headers,
        plan::{DEFAULT_REQUEST_TIMEOUT, WorkloadPlan},
    },
    workspace::{self, command},
};

// Rejects invalid configuration before anything is built or started
pub fn validate(request: &BenchmarkRequest) -> Result<(), String> {
    validate_build(&request.build)?;

    if let Some(cleanup) = &request.cleanup {
        cleanup::validate(cleanup, "cleanup")?;
    }

    if let Some(startup) = &request.startup {
        validate_startup(startup)?;
    }

    if let Some(workload) = &request.workload {
        // Host and port only matter at runtime, the rest is checked here
        WorkloadPlan::new(workload, "127.0.0.1", 1, DEFAULT_REQUEST_TIMEOUT)?;
    }

    workspace::validation::validate_all(&request.workspaces)?;

    Ok(())
}

fn validate_startup(startup: &StartupRequest) -> Result<(), String> {
    if startup.command.program.trim().is_empty() {
        return Err("startup.command.program must not be empty".to_string());
    }

    let readiness = &startup.readiness;

    if readiness.readiness_type != "http" {
        return Err(format!(
            "Unsupported readiness type: {} (only http is supported)",
            readiness.readiness_type
        ));
    }

    if !readiness.method.eq_ignore_ascii_case("GET") {
        return Err(format!(
            "Unsupported readiness method: {} (only GET is supported)",
            readiness.method
        ));
    }

    if !is_loopback(&readiness.host) {
        return Err(format!(
            "Readiness host must be loopback (127.0.0.1, localhost or ::1): {}",
            readiness.host
        ));
    }

    if readiness.port == 0 {
        return Err("Readiness port must be greater than 0".to_string());
    }

    if !readiness.path.starts_with('/')
        || readiness
            .path
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err("Readiness path must start with '/' and contain no whitespace".to_string());
    }

    if !(100..=599).contains(&readiness.expected_status) {
        return Err("Readiness expected_status must be between 100 and 599".to_string());
    }

    if let Some(readiness_headers) = &readiness.headers {
        headers::parse(readiness_headers)?;
    }

    Ok(())
}

fn validate_build(build: &BuildRequest) -> Result<(), String> {
    if build.command.trim().is_empty() {
        return Err("build.command must not be empty".to_string());
    }

    command::split(&build.command).map_err(|message| format!("build.command: {message}"))?;

    if let Some(output_dir) = build.output_dir.as_deref() {
        validate_output_dir(output_dir)?;
    }

    Ok(())
}

// Empty means the runtime default. Anything else must stay inside the project.
fn validate_output_dir(output_dir: &str) -> Result<(), String> {
    let output_dir = output_dir.trim();

    if output_dir.is_empty() {
        return Ok(());
    }

    if output_dir.chars().any(char::is_control) {
        return Err("build.output_dir must not contain control characters".to_string());
    }

    let path = Path::new(output_dir);

    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(
            "build.output_dir must be a path inside the project, without \"..\"".to_string(),
        );
    }

    Ok(())
}
