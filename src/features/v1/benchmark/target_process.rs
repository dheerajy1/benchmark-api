use std::{
    path::Path,
    process::{Child, Command},
    thread,
    time::{Duration, Instant, SystemTime},
};

use crate::config::env::Env;
use crate::features::v1::benchmark::{
    live::RunControl,
    model::BuildCommand,
    process_group,
    readiness::{self, ReadinessCheck, ReadinessSettings},
};

pub const READINESS_PATH: &str = "/api/v1/health/app";
pub const READINESS_HOST: &str = "127.0.0.1";
pub const READINESS_METHOD: &str = "GET";
pub const EXPECTED_STATUS: u16 = 200;

const POLL_INTERVAL: Duration = Duration::from_millis(50);

// Only these variables are passed from benchmark-api to the target app
const PASSTHROUGH_ENV: [&str; 8] = [
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "TMPDIR",
    "SystemRoot",
    "USERPROFILE",
    "TEMP",
];

#[derive(Debug, PartialEq, Eq)]
pub enum ReadyOutcome {
    // Server answered with the expected status
    Ready,
    // Server answered with any other status
    Rejected,
    // Process exited before becoming ready
    Exited(Option<i32>),
    // Readiness was not reached within the timeout
    TimedOut,
    // A cancel request ended the wait, the process is still running
    Cancelled,
}

pub struct TargetProcess {
    child: Child,
    started: Instant,
    started_at: SystemTime,
    ready_elapsed: Option<Duration>,
    shut_down: bool,
}

impl TargetProcess {
    pub fn start(command: &BuildCommand, working_directory: &Path) -> Result<Self, String> {
        // Timing begins immediately before spawn so startup duration is unchanged
        let started_at = SystemTime::now();
        let started = Instant::now();
        let child = spawn_start_process(command, working_directory)?;

        Ok(Self {
            child,
            started,
            started_at,
            ready_elapsed: None,
            shut_down: false,
        })
    }

    // Wall-clock time captured immediately before spawn
    pub fn started_at(&self) -> SystemTime {
        self.started_at
    }

    // Waits using the default readiness settings for a discovered port
    pub fn wait_until_ready(
        &mut self,
        port: u16,
        env: &Env,
        timeout: Duration,
    ) -> Result<ReadyOutcome, String> {
        self.wait_until_ready_with(&ReadinessSettings::discovered(port), env, timeout)
    }

    pub fn wait_until_ready_with(
        &mut self,
        settings: &ReadinessSettings,
        env: &Env,
        timeout: Duration,
    ) -> Result<ReadyOutcome, String> {
        self.wait_until_ready_with_control(settings, env, timeout, &RunControl::default())
    }

    // Same wait, ended early by a cancel request. Each probe is capped at about 200 ms and the
    // pause between probes is 50 ms, so a request is seen within about 250 ms.
    pub fn wait_until_ready_with_control(
        &mut self,
        settings: &ReadinessSettings,
        env: &Env,
        timeout: Duration,
        control: &RunControl,
    ) -> Result<ReadyOutcome, String> {
        let outcome = loop {
            if control.is_cancel_requested() {
                break ReadyOutcome::Cancelled;
            }

            match readiness::check(settings, env) {
                ReadinessCheck::Ready => break ReadyOutcome::Ready,
                ReadinessCheck::Rejected => break ReadyOutcome::Rejected,
                ReadinessCheck::NotReady => {}
            }

            match self.child.try_wait() {
                Ok(Some(status)) => break ReadyOutcome::Exited(status.code()),
                Ok(None) => {}
                Err(error) => {
                    return Err(format!("Failed to inspect startup process: {error}"));
                }
            }

            if self.started.elapsed() >= timeout {
                break ReadyOutcome::TimedOut;
            }

            thread::sleep(POLL_INTERVAL);
        };

        self.ready_elapsed = Some(self.started.elapsed());

        Ok(outcome)
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    // Process group id, equal to the leader pid because the target runs under setsid
    pub fn process_group_id(&self) -> u32 {
        self.child.id()
    }

    // Time from spawn until readiness reached an outcome
    pub fn startup_elapsed(&self) -> Duration {
        self.ready_elapsed.unwrap_or_else(|| self.started.elapsed())
    }

    // Safe to call more than once, only the first call terminates. Best effort: a group that
    // could not be stopped is reported on stderr and the benchmark result is not changed by it.
    pub fn shutdown(&mut self) {
        if let Err(message) = self.shutdown_verified() {
            eprintln!("warning: the target could not be stopped completely, {message}");
        }
    }

    // Stops the whole process group and checks that nothing of it is left running. Only the
    // first call acts, later calls answer Ok.
    pub fn shutdown_verified(&mut self) -> Result<(), String> {
        if self.shut_down {
            return Ok(());
        }

        self.shut_down = true;

        process_group::stop_child_group(&mut self.child)
    }
}

impl Drop for TargetProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn apply_clean_env(process: &mut Command) {
    // Drop everything inherited from benchmark-api
    process.env_clear();

    for key in PASSTHROUGH_ENV {
        // Copy the variable only if it is set in the parent
        if let Some(value) = std::env::var_os(key) {
            process.env(key, value);
        }
    }
}

fn spawn_start_process(command: &BuildCommand, working_directory: &Path) -> Result<Child, String> {
    #[cfg(unix)]
    {
        let mut process = Command::new("setsid");

        process
            .arg(&command.program)
            .args(&command.args)
            .current_dir(working_directory);

        apply_clean_env(&mut process);

        process
            .spawn()
            .map_err(|error| format!("Failed to spawn startup command: {error}"))
    }

    #[cfg(not(unix))]
    {
        let mut process = Command::new(&command.program);

        process.args(&command.args).current_dir(working_directory);

        apply_clean_env(&mut process);

        process
            .spawn()
            .map_err(|error| format!("Failed to spawn startup command: {error}"))
    }
}
