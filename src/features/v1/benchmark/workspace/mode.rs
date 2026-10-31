use std::{collections::BTreeMap, path::Path};

use crate::features::v1::benchmark::{
    live::{RunContext, RunControl},
    metrics::memory::MemoryConfig,
    stop::Stop,
};

use super::{
    config::WorkspaceRequest,
    result::{STATUS_FAILED, STATUS_SUCCESS, STATUS_TIMEOUT, WorkspaceResult},
    sampling::{Activity, Window},
};

// Everything the two modes of one workspace share
pub struct Shared<'a> {
    pub workspace: &'a WorkspaceRequest,
    // The workspace folder, already checked to exist
    pub directory: &'a Path,
    pub variables: &'a BTreeMap<String, String>,
    pub variable_names: &'a [String],
    // True when the workspace has an env, even an empty one
    pub has_env: bool,
    pub memory: &'a MemoryConfig,
    pub control: &'a RunControl,
    // Where the live events of the run go
    pub ctx: &'a RunContext,
}

impl Shared<'_> {
    pub fn activity<'b>(
        &'b self,
        load: Option<&'b crate::features::v1::benchmark::model::BuildCommand>,
        mode: &'b str,
    ) -> Activity<'b> {
        Activity {
            ctx: self.ctx,
            workspace: &self.workspace.name,
            mode,
            load,
            directory: self.directory,
            env: self.variables,
            idle_seconds: self.workspace.idle_seconds,
        }
    }
}

// A mode that could not be measured keeps its status and null numbers
pub fn fail(result: &mut WorkspaceResult, message: String) {
    result.status = STATUS_FAILED.to_string();
    result.message = Some(message);
}

pub fn time_out(result: &mut WorkspaceResult, message: String) {
    result.status = STATUS_TIMEOUT.to_string();
    result.message = Some(message);
}

// A cancel request goes up, any other stop is the mode's own failure
pub fn finish(
    mut result: WorkspaceResult,
    outcome: Result<(), Stop>,
) -> Result<WorkspaceResult, Stop> {
    match outcome {
        Ok(()) => Ok(result),
        Err(Stop::Cancelled) => Err(Stop::Cancelled),
        Err(Stop::Failed(message)) => {
            fail(&mut result, message);

            Ok(result)
        }
    }
}

// Combines how the measurement ended with how the process was stopped. A cancelled run is only
// reported as cancelled when everything is really gone, any other ending keeps its result and
// reports what could not be stopped on stderr.
pub fn settle(outcome: Result<(), Stop>, shutdown: Result<(), String>) -> Result<(), Stop> {
    match (outcome, shutdown) {
        (Err(Stop::Cancelled), Err(message)) => Err(Stop::Failed(format!(
            "The run was cancelled but a process could not be stopped completely: {message}"
        ))),
        (outcome, Err(message)) => {
            eprintln!("warning: a process could not be stopped completely, {message}");

            outcome
        }
        (outcome, Ok(())) => outcome,
    }
}

// Stores what the sampling window measured. Real numbers are kept even when the window did not
// end cleanly, the status says what went wrong.
pub fn apply_window(result: &mut WorkspaceResult, window: Window) {
    result.memory = Some(window.memory);
    result.cpu = window.cpu;
    result.idle_seconds = window.idle_seconds;

    let load_failed = window
        .load
        .as_ref()
        .filter(|load| load.status != STATUS_SUCCESS);

    if let Some(load) = load_failed {
        result.message = Some(match load.exit_code {
            Some(code) => format!("The load command failed with exit code {code}"),
            None => format!("The load command did not succeed ({})", load.status),
        });
        result.status = STATUS_FAILED.to_string();
    } else if window.target_gone {
        result.message = Some("The process ended while it was being measured".to_string());
        result.status = STATUS_FAILED.to_string();
    } else {
        result.status = STATUS_SUCCESS.to_string();
    }

    result.load = window.load;
}
