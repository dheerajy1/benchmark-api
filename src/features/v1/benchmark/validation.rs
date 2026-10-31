use super::{
    host::is_loopback,
    model::BenchmarkRequest,
    request::StartupRequest,
    workload::{
        headers,
        plan::{DEFAULT_REQUEST_TIMEOUT, WorkloadPlan},
    },
    workspace,
};

// Rejects invalid configuration before anything is built or started
pub fn validate(request: &BenchmarkRequest) -> Result<(), String> {
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
