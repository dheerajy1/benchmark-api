use std::{path::Path, time::Duration};

use super::{
    metrics::build_time,
    model::{BenchmarkRequest, BuildCommand, CleanupRequest},
    workspace::command,
};

// How long a cleanup command may run when the request gives no timeout
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 120;

// The longest timeout a request may ask for
pub const MAX_TIMEOUT_SECONDS: u64 = 3600;

// The two kinds of cleanup step of a run
pub const STEP_BUILD: &str = "build";
pub const STEP_DOCKER: &str = "docker";

// One cleanup command that is set, ready to run
pub struct CleanupStep {
    // "build" for the one next to the build, "docker" for one of a Docker workspace
    pub step: &'static str,
    // The workspace a docker step belongs to
    pub workspace: Option<String>,
    pub command: BuildCommand,
    pub timeout: Duration,
}

impl CleanupStep {
    // The key the stage records use, one row for each step
    pub fn stage_key(&self) -> String {
        match &self.workspace {
            Some(workspace) => format!("cleanup:{}:{workspace}", self.step),
            None => format!("cleanup:{}", self.step),
        }
    }
}

// How one cleanup command ended
pub struct CleanupOutcome {
    // "success", "failed" or "timeout"
    pub status: String,
    pub duration_ms: Option<u64>,
    // Why it did not succeed
    pub message: Option<String>,
}

impl CleanupOutcome {
    pub fn succeeded(&self) -> bool {
        self.status == "success"
    }
}

// An empty command means no cleanup, so a form that leaves the field blank changes nothing
pub fn is_set(cleanup: &CleanupRequest) -> bool {
    !cleanup.command.trim().is_empty()
}

// `field` names the request field in the message, for example "cleanup" or "docker.cleanup"
pub fn validate(cleanup: &CleanupRequest, field: &str) -> Result<(), String> {
    if !is_set(cleanup) {
        return Ok(());
    }

    command::split(&cleanup.command).map_err(|message| format!("{field}.command: {message}"))?;

    if let Some(seconds) = cleanup.timeout_seconds
        && !(1..=MAX_TIMEOUT_SECONDS).contains(&seconds)
    {
        return Err(format!(
            "{field}.timeout_seconds must be between 1 and {MAX_TIMEOUT_SECONDS}"
        ));
    }

    Ok(())
}

fn step_of(
    cleanup: &CleanupRequest,
    step: &'static str,
    workspace: Option<String>,
    field: &str,
) -> Result<CleanupStep, String> {
    validate(cleanup, field)?;

    Ok(CleanupStep {
        step,
        workspace,
        command: command::split(&cleanup.command)
            .map_err(|message| format!("{field}.command: {message}"))?,
        timeout: Duration::from_secs(cleanup.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS)),
    })
}

// The cleanup commands that are set, in the order they run: the build's first, then the Docker
// ones. A command that is empty, or a Docker mode that is off, leaves its step out.
pub fn plan(request: &BenchmarkRequest) -> Result<Vec<CleanupStep>, String> {
    let mut steps = Vec::new();

    if let Some(cleanup) = request.cleanup.as_ref().filter(|cleanup| is_set(cleanup)) {
        steps.push(step_of(cleanup, STEP_BUILD, None, "cleanup")?);
    }

    for workspace in &request.workspaces {
        let Some(cleanup) = workspace
            .docker
            .as_ref()
            .filter(|docker| docker.enabled)
            .and_then(|docker| docker.cleanup.as_ref())
            .filter(|cleanup| is_set(cleanup))
        else {
            continue;
        };

        steps.push(step_of(
            cleanup,
            STEP_DOCKER,
            Some(workspace.name.clone()),
            "docker.cleanup",
        )?);
    }

    Ok(steps)
}

// Whether the request has any cleanup command to run
pub fn is_requested(request: &BenchmarkRequest) -> bool {
    plan(request).is_ok_and(|steps| !steps.is_empty())
}

// Runs one command in the project path. It is not stopped by a cancel request, because a
// cancelled run has the most to clean up.
pub fn run(target: &Path, step: &CleanupStep) -> CleanupOutcome {
    match build_time::measure_with_timeout(target, &step.command, step.timeout) {
        Err(message) => CleanupOutcome {
            status: "failed".to_string(),
            duration_ms: None,
            message: Some(format!("The cleanup command could not run: {message}")),
        },
        Ok(measured) => {
            let message = match measured.status.as_str() {
                "success" => None,
                "timeout" => Some(format!(
                    "The cleanup command did not finish within {} seconds.",
                    step.timeout.as_secs()
                )),
                _ => Some(match measured.exit_code {
                    Some(code) => format!("The cleanup command failed with exit code {code}."),
                    None => "The cleanup command failed.".to_string(),
                }),
            };

            CleanupOutcome {
                status: measured.status,
                duration_ms: measured.duration_ms,
                message,
            }
        }
    }
}
