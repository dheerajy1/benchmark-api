use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use uuid::Uuid;

use crate::features::v1::benchmark::{
    docker::Docker,
    model::BuildCommand,
    stop::{Stop, sleep_unless_cancelled},
};

use super::{
    command,
    config::{DockerRequest, ENV_FILE_PLACEHOLDER},
    exec::{self, Finished, LineOutcome, Process, Spec},
    mode::{self, Shared},
    native::build_status,
    result::{MODE_DOCKER, WorkspaceBuild, WorkspaceResult, WorkspaceStartup},
    sampling::{self, Scope},
};

const POLL: Duration = Duration::from_millis(50);

// Measures the workspace as a container. The memory is read from the host, from the
// container's main process and its children, the same way the native process is read.
pub fn run(
    shared: &Shared<'_>,
    request: &DockerRequest,
    docker: &dyn Docker,
) -> Result<WorkspaceResult, Stop> {
    let mut result = WorkspaceResult::empty(
        &shared.workspace.name,
        MODE_DOCKER,
        shared.variable_names.to_vec(),
    );

    let outcome = measure(shared, request, docker, &mut result);

    mode::finish(result, outcome)
}

fn measure(
    shared: &Shared<'_>,
    request: &DockerRequest,
    docker: &dyn Docker,
    result: &mut WorkspaceResult,
) -> Result<(), Stop> {
    let build = command::split(&request.build_command).map_err(Stop::Failed)?;

    let timed = exec::run_to_completion(
        &Spec {
            command: &build,
            directory: shared.directory,
            env: shared.variables,
        },
        Duration::from_secs(request.build_timeout_seconds),
        shared.control,
    )?;

    result.build = Some(WorkspaceBuild {
        status: build_status(&timed).to_string(),
        duration_ms: timed.duration_ms(),
        exit_code: timed.exit_code(),
    });

    match timed.finished {
        Finished::Exited(Some(0)) => {}
        Finished::Exited(code) => {
            mode::fail(
                result,
                match code {
                    Some(code) => format!("The image build failed with exit code {code}"),
                    None => "The image build was ended by a signal".to_string(),
                },
            );

            return Ok(());
        }
        Finished::TimedOut => {
            mode::time_out(
                result,
                format!(
                    "The image build did not finish within {} seconds",
                    request.build_timeout_seconds
                ),
            );

            return Ok(());
        }
    }

    let bytes = docker
        .image_size_bytes(&request.image)
        .map_err(Stop::Failed)?;
    // Only a nicety, a Docker that cannot answer never fails the mode
    let disk_usage = docker.image_disk_usage_bytes(&request.image).ok().flatten();
    result.image = Some(sampling::size_of(bytes, disk_usage));

    // The variables only ever reach the container through this file, which only the owner can
    // read and which is deleted when this function ends
    let env_file = if shared.has_env {
        Some(EnvFile::create(shared.variables).map_err(Stop::Failed)?)
    } else {
        None
    };

    let run_command =
        prepare_run_command(&request.run_command, env_file.as_ref().map(EnvFile::path))
            .map_err(Stop::Failed)?;

    let load = shared
        .workspace
        .load
        .as_ref()
        .map(|load| command::split(&load.command))
        .transpose()
        .map_err(Stop::Failed)?;

    // A stopped container that still holds the name would make the run command fail
    docker
        .stop_and_remove(&request.container_name)
        .map_err(Stop::Failed)?;

    let outcome = run_container(shared, request, docker, &run_command, load.as_ref(), result);

    // The container is removed on success and on every error path
    let removal = docker.stop_and_remove(&request.container_name);

    drop(env_file);

    mode::settle(outcome, removal)
}

fn run_container(
    shared: &Shared<'_>,
    request: &DockerRequest,
    docker: &dyn Docker,
    run_command: &BuildCommand,
    load: Option<&BuildCommand>,
    result: &mut WorkspaceResult,
) -> Result<(), Stop> {
    let no_env = BTreeMap::new();
    let started = Instant::now();

    // The run command's own output is discarded, readiness comes from `docker logs`, which
    // also works when the command starts the container detached
    let mut run_process = Process::start(
        &Spec {
            command: run_command,
            directory: shared.directory,
            env: &no_env,
        },
        false,
    )
    .map_err(Stop::Failed)?;

    let outcome = container_phases(
        shared,
        request,
        docker,
        &mut run_process,
        started,
        load,
        result,
    );

    let shutdown = run_process.shutdown_verified();

    mode::settle(outcome, shutdown)
}

fn container_phases(
    shared: &Shared<'_>,
    request: &DockerRequest,
    docker: &dyn Docker,
    run_process: &mut Process,
    started: Instant,
    load: Option<&BuildCommand>,
    result: &mut WorkspaceResult,
) -> Result<(), Stop> {
    let no_env = BTreeMap::new();
    let deadline = started + Duration::from_secs(shared.workspace.ready_timeout_seconds);

    // The container's main process id on the host
    let pid = loop {
        if shared.control.is_cancel_requested() {
            return Err(Stop::Cancelled);
        }

        if let Some(pid) = docker
            .container_pid(&request.container_name)
            .map_err(Stop::Failed)?
        {
            break pid;
        }

        if let Some(code) = run_process.exit_code()
            && code != Some(0)
        {
            mode::fail(
                result,
                match code {
                    Some(code) => format!("The run command failed with exit code {code}"),
                    None => "The run command was ended by a signal".to_string(),
                },
            );

            return Ok(());
        }

        if Instant::now() >= deadline {
            mode::time_out(
                result,
                format!(
                    "The container did not start within {} seconds",
                    shared.workspace.ready_timeout_seconds
                ),
            );

            return Ok(());
        }

        if sleep_unless_cancelled(POLL, shared.control) {
            return Err(Stop::Cancelled);
        }
    };

    let logs_command = docker.logs_command(&request.container_name);

    let mut logs = Process::start(
        &Spec {
            command: &logs_command,
            directory: shared.directory,
            env: &no_env,
        },
        true,
    )
    .map_err(Stop::Failed)?;

    let line = logs.wait_for_line(&shared.workspace.ready_log_line, deadline, shared.control);

    // The log follower is not needed after this point
    if let Err(message) = logs.shutdown_verified() {
        eprintln!("warning: the log follower could not be stopped completely, {message}");
    }

    match line.map_err(Stop::Failed)? {
        LineOutcome::Found => {
            result.startup = Some(WorkspaceStartup {
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            });
        }
        LineOutcome::Exited(_) => {
            mode::fail(
                result,
                "The container stopped before it printed the ready line".to_string(),
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
        Scope::Tree(pid),
        shared.memory,
        &shared.activity(load, MODE_DOCKER),
        shared.control,
    )?;

    mode::apply_window(result, window);

    Ok(())
}

// The run command with the placeholder replaced by the env file path, argument by argument so
// a path with spaces stays one argument
pub fn prepare_run_command(
    template: &str,
    env_file: Option<&Path>,
) -> Result<BuildCommand, String> {
    let mut command = command::split(template)?;

    let Some(path) = env_file else {
        return Ok(command);
    };

    let path = path.display().to_string();

    command.program = command.program.replace(ENV_FILE_PLACEHOLDER, &path);

    for argument in &mut command.args {
        *argument = argument.replace(ENV_FILE_PLACEHOLDER, &path);
    }

    Ok(command)
}

// The temporary file Docker reads the variables from. Owner-only permissions, removed on drop.
pub struct EnvFile {
    path: PathBuf,
}

impl EnvFile {
    pub fn create(variables: &BTreeMap<String, String>) -> Result<Self, String> {
        let mut body = String::new();

        for (name, value) in variables {
            // A Docker env file has one variable per line and no way to escape a line break
            if value.contains('\n') || value.contains('\r') {
                return Err(format!(
                    "The value of {name} has a line break, a Docker env file cannot hold it"
                ));
            }

            body.push_str(name);
            body.push('=');
            body.push_str(value);
            body.push('\n');
        }

        let path = std::env::temp_dir().join(format!("benchmark-workspace-{}.env", Uuid::now_v7()));

        let mut options = OpenOptions::new();

        options.write(true).create_new(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;

            options.mode(0o600);
        }

        let mut file = options
            .open(&path)
            .map_err(|error| format!("Failed to create the temporary env file: {error}"))?;

        // The file exists now, so it is owned by the guard before anything can fail
        let guard = Self { path };

        file.write_all(body.as_bytes())
            .map_err(|error| format!("Failed to write the temporary env file: {error}"))?;

        Ok(guard)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for EnvFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
