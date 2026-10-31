use std::time::{Duration, Instant};

use crate::features::v1::benchmark::{model::BuildCommand, stop::Stop};

use super::{
    command,
    config::{DEFAULT_BUILD_TIMEOUT_SECONDS, NativeRequest},
    exec::{self, Finished, LineOutcome, Process, Spec},
    mode::{self, Shared},
    result::{MODE_NATIVE, WorkspaceBuild, WorkspaceResult, WorkspaceStartup},
    sampling::{self, Scope},
};

// Measures the workspace as a plain process on this machine
pub fn run(shared: &Shared<'_>, native: &NativeRequest) -> Result<WorkspaceResult, Stop> {
    let mut result = WorkspaceResult::empty(
        &shared.workspace.name,
        MODE_NATIVE,
        shared.variable_names.to_vec(),
    );

    let outcome = measure(shared, native, &mut result);

    mode::finish(result, outcome)
}

fn measure(
    shared: &Shared<'_>,
    native: &NativeRequest,
    result: &mut WorkspaceResult,
) -> Result<(), Stop> {
    if let Some(build) = native.build_command.as_deref() {
        let build = command::split(build).map_err(Stop::Failed)?;

        if !build_native(shared, &build, result)? {
            return Ok(());
        }
    }

    let start = command::split(&native.command).map_err(Stop::Failed)?;
    let load = shared
        .workspace
        .load
        .as_ref()
        .map(|load| command::split(&load.command))
        .transpose()
        .map_err(Stop::Failed)?;

    let spec = Spec {
        command: &start,
        directory: shared.directory,
        env: shared.variables,
    };

    let started = Instant::now();
    let mut process = Process::start(&spec, true).map_err(Stop::Failed)?;

    let outcome = phases(shared, &mut process, started, load.as_ref(), result);

    // Shutdown happens once, on success and on every error path
    let shutdown = process.shutdown_verified();

    mode::settle(outcome, shutdown)
}

// Answers whether the build succeeded, a failed build is recorded and ends the mode
fn build_native(
    shared: &Shared<'_>,
    build: &BuildCommand,
    result: &mut WorkspaceResult,
) -> Result<bool, Stop> {
    let timed = exec::run_to_completion(
        &Spec {
            command: build,
            directory: shared.directory,
            env: shared.variables,
        },
        Duration::from_secs(DEFAULT_BUILD_TIMEOUT_SECONDS),
        shared.control,
    )?;

    result.build = Some(WorkspaceBuild {
        status: build_status(&timed).to_string(),
        duration_ms: timed.duration_ms(),
        exit_code: timed.exit_code(),
    });

    match timed.finished {
        Finished::Exited(Some(0)) => Ok(true),
        Finished::Exited(code) => {
            mode::fail(
                result,
                match code {
                    Some(code) => format!("The native build failed with exit code {code}"),
                    None => "The native build was ended by a signal".to_string(),
                },
            );

            Ok(false)
        }
        Finished::TimedOut => {
            mode::time_out(
                result,
                format!(
                    "The native build did not finish within {DEFAULT_BUILD_TIMEOUT_SECONDS} seconds"
                ),
            );

            Ok(false)
        }
    }
}

pub fn build_status(timed: &exec::Timed) -> &'static str {
    match timed.finished {
        Finished::Exited(Some(0)) => "success",
        Finished::Exited(_) => "failed",
        Finished::TimedOut => "timeout",
    }
}

fn phases(
    shared: &Shared<'_>,
    process: &mut Process,
    started: Instant,
    load: Option<&BuildCommand>,
    result: &mut WorkspaceResult,
) -> Result<(), Stop> {
    let timeout = Duration::from_secs(shared.workspace.ready_timeout_seconds);

    let line = process
        .wait_for_line(
            &shared.workspace.ready_log_line,
            started + timeout,
            shared.control,
        )
        .map_err(Stop::Failed)?;

    match line {
        LineOutcome::Found => {
            result.startup = Some(WorkspaceStartup {
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            });
        }
        LineOutcome::Exited(code) => {
            mode::fail(
                result,
                match code {
                    Some(code) => {
                        format!("The process exited with code {code} before it was ready")
                    }
                    None => "The process was ended by a signal before it was ready".to_string(),
                },
            );

            return Ok(());
        }
        LineOutcome::TimedOut => {
            mode::time_out(
                result,
                format!(
                    "The ready line was not seen within {} seconds",
                    shared.workspace.ready_timeout_seconds
                ),
            );

            return Ok(());
        }
        LineOutcome::Cancelled => return Err(Stop::Cancelled),
    }

    let window = sampling::measure(
        Scope::Group(process.process_group_id()),
        shared.memory,
        &shared.activity(load, MODE_NATIVE),
        shared.control,
    )?;

    mode::apply_window(result, window);

    Ok(())
}
