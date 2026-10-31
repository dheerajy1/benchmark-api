use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use crate::features::v1::benchmark::{
    live::RunControl, model::BuildCommand, process_group, stop::Stop,
};

// Only these variables are passed from benchmark-api to a workspace process. The Docker ones
// let a docker command reach the same daemon and settings the person's shell would.
const PASSTHROUGH_ENV: [&str; 12] = [
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "TMPDIR",
    "DOCKER_HOST",
    "DOCKER_CONFIG",
    "DOCKER_CONTEXT",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "SystemRoot",
    "TEMP",
];

const POLL_INTERVAL: Duration = Duration::from_millis(20);
// Lines the process wrote just before it exited may still be on their way
const EXIT_DRAIN: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finished {
    // The exit code, None when a signal ended the process
    Exited(Option<i32>),
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timed {
    pub finished: Finished,
    pub duration: Duration,
}

impl Timed {
    pub fn succeeded(&self) -> bool {
        self.finished == Finished::Exited(Some(0))
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self.finished {
            Finished::Exited(code) => code,
            Finished::TimedOut => None,
        }
    }

    pub fn duration_ms(&self) -> u64 {
        u64::try_from(self.duration.as_millis()).unwrap_or(u64::MAX)
    }
}

// Where and with what a command runs
pub struct Spec<'a> {
    pub command: &'a BuildCommand,
    pub directory: &'a Path,
    // Added to the passed-through variables, never printed
    pub env: &'a BTreeMap<String, String>,
}

// Runs a command until it ends, its output is discarded. A timeout or a cancel request stops
// its whole process group and checks that nothing of it is left.
pub fn run_to_completion(
    spec: &Spec<'_>,
    timeout: Duration,
    control: &RunControl,
) -> Result<Timed, Stop> {
    let started = Instant::now();
    let mut child = spawn(spec, Stdio::null(), Stdio::null())?;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(Timed {
                    finished: Finished::Exited(status.code()),
                    duration: started.elapsed(),
                });
            }
            Ok(None) => {}
            Err(error) => {
                let _ = process_group::stop_child_group(&mut child);

                return Err(Stop::Failed(format!(
                    "Failed to inspect a command: {error}"
                )));
            }
        }

        if control.is_cancel_requested() {
            return match process_group::stop_child_group(&mut child) {
                Ok(()) => Err(Stop::Cancelled),
                Err(message) => Err(Stop::Failed(format!(
                    "The run was cancelled but a command could not be stopped completely: {message}"
                ))),
            };
        }

        if started.elapsed() >= timeout {
            // The timeout result stands even when stopping is incomplete
            if let Err(message) = process_group::stop_child_group(&mut child) {
                eprintln!("warning: a command could not be stopped completely, {message}");
            }

            return Ok(Timed {
                finished: Finished::TimedOut,
                duration: started.elapsed(),
            });
        }

        thread::sleep(POLL_INTERVAL);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOutcome {
    // A line with the text appeared
    Found,
    // The process ended without printing it
    Exited(Option<i32>),
    TimedOut,
    Cancelled,
}

// A running process in its own process group. With `capture` its output is read line by line
// so readiness can be a log line, otherwise the output is discarded.
pub struct Process {
    child: Child,
    lines: Option<Receiver<String>>,
    shut_down: bool,
}

impl Process {
    pub fn start(spec: &Spec<'_>, capture: bool) -> Result<Self, String> {
        let (stdout, stderr) = if capture {
            (Stdio::piped(), Stdio::piped())
        } else {
            (Stdio::null(), Stdio::null())
        };

        let mut child = spawn(spec, stdout, stderr)?;

        let lines = capture.then(|| {
            let (sender, receiver) = mpsc::channel::<String>();

            if let Some(output) = child.stdout.take() {
                forward_lines(output, sender.clone());
            }

            if let Some(output) = child.stderr.take() {
                forward_lines(output, sender);
            }

            receiver
        });

        Ok(Self {
            child,
            lines,
            shut_down: false,
        })
    }

    // Process group id, equal to the pid because the process runs under setsid
    pub fn process_group_id(&self) -> u32 {
        self.child.id()
    }

    pub fn exit_code(&mut self) -> Option<Option<i32>> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code()),
            _ => None,
        }
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    // Waits until a line contains the text. Output keeps being read afterwards by the reader
    // threads and thrown away, so the process never blocks on a full pipe.
    pub fn wait_for_line(
        &mut self,
        text: &str,
        deadline: Instant,
        control: &RunControl,
    ) -> Result<LineOutcome, String> {
        let Some(lines) = self.lines.take() else {
            return Err("The process output is not captured".to_string());
        };

        let outcome = self.scan(&lines, text, deadline, control);

        // Dropping the receiver makes the reader threads discard what they read
        drop(lines);

        outcome
    }

    fn scan(
        &mut self,
        lines: &Receiver<String>,
        text: &str,
        deadline: Instant,
        control: &RunControl,
    ) -> Result<LineOutcome, String> {
        loop {
            if control.is_cancel_requested() {
                return Ok(LineOutcome::Cancelled);
            }

            match lines.recv_timeout(POLL_INTERVAL) {
                Ok(line) => {
                    if line.contains(text) {
                        return Ok(LineOutcome::Found);
                    }

                    continue;
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }

            match self.child.try_wait() {
                Ok(Some(status)) => {
                    // Lines written just before the exit still count
                    let drain_until = Instant::now() + EXIT_DRAIN;

                    while Instant::now() < drain_until {
                        match lines.recv_timeout(POLL_INTERVAL) {
                            Ok(line) if line.contains(text) => return Ok(LineOutcome::Found),
                            Ok(_) => {}
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }

                    return Ok(LineOutcome::Exited(status.code()));
                }
                Ok(None) => {}
                Err(error) => return Err(format!("Failed to inspect a process: {error}")),
            }

            if Instant::now() >= deadline {
                return Ok(LineOutcome::TimedOut);
            }
        }
    }

    // Stops the whole process group and checks that nothing of it is left. Only the first
    // call acts.
    pub fn shutdown_verified(&mut self) -> Result<(), String> {
        if self.shut_down {
            return Ok(());
        }

        self.shut_down = true;

        process_group::stop_child_group(&mut self.child)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Err(message) = self.shutdown_verified() {
            eprintln!("warning: a process could not be stopped completely, {message}");
        }
    }
}

fn forward_lines<R: Read + Send + 'static>(output: R, sender: Sender<String>) {
    thread::spawn(move || {
        for line in BufReader::new(output).split(b'\n') {
            let Ok(bytes) = line else {
                break;
            };

            // A closed receiver only means nobody is waiting for lines any more, reading
            // goes on so the pipe never fills
            let _ = sender.send(String::from_utf8_lossy(&bytes).into_owned());
        }
    });
}

fn spawn(spec: &Spec<'_>, stdout: Stdio, stderr: Stdio) -> Result<Child, String> {
    let mut process = if cfg!(unix) {
        let mut process = Command::new("setsid");

        process.arg(&spec.command.program);
        process
    } else {
        Command::new(&spec.command.program)
    };

    process
        .args(&spec.command.args)
        .current_dir(spec.directory)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .env_clear();

    for key in PASSTHROUGH_ENV {
        if let Some(value) = std::env::var_os(key) {
            process.env(key, value);
        }
    }

    process.envs(spec.env);

    process
        .spawn()
        .map_err(|error| format!("Failed to start '{}': {error}", spec.command.program))
}
