use std::{
    path::Path,
    process::{Child, Command},
    thread,
    time::{Duration, Instant, SystemTime},
};

use crate::config::env::Env;
use crate::features::v1::benchmark::model::BuildCommand;

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
}

enum ReadinessCheck {
    // Server answered with the expected status
    Ready,
    // Server answered with any other status
    Rejected,
    // No usable answer yet, keep polling
    NotReady,
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
    pub fn wait_until_ready(
        &mut self,
        port: u16,
        env: &Env,
        timeout: Duration,
    ) -> Result<ReadyOutcome, String> {
        let outcome = loop {
            match http_ready(READINESS_HOST, port, env) {
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

    // Safe to call more than once, only the first call terminates
    pub fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }

        self.shut_down = true;

        terminate_process_tree(&mut self.child);
        let _ = self.child.wait();
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

fn http_ready(host: &str, port: u16, env: &Env) -> ReadinessCheck {
    use std::{
        io::{Read, Write},
        net::TcpStream,
    };

    let Ok(mut stream) = TcpStream::connect_timeout(
        &format!("{host}:{port}").parse().unwrap(),
        Duration::from_millis(100),
    ) else {
        return ReadinessCheck::NotReady;
    };

    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));

    let request = format!(
        "GET {READINESS_PATH} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         X-Client-Id: {}\r\n\
         X-Client-Secret: {}\r\n\
         Connection: close\r\n\r\n",
        env.app_health_client_id, env.app_health_client_secret
    );

    if stream.write_all(request.as_bytes()).is_err() {
        return ReadinessCheck::NotReady;
    }

    let mut response = [0u8; 1024];

    let Ok(bytes_read) = stream.read(&mut response) else {
        return ReadinessCheck::NotReady;
    };

    let response = String::from_utf8_lossy(&response[..bytes_read]);

    let Some(status_line) = response.lines().next() else {
        return ReadinessCheck::NotReady;
    };

    let mut parts = status_line.split_whitespace();

    let Some(version) = parts.next() else {
        return ReadinessCheck::NotReady;
    };

    let Some(status) = parts.next() else {
        return ReadinessCheck::NotReady;
    };

    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return ReadinessCheck::NotReady;
    }

    match status.parse::<u16>() {
        Ok(code) if code == EXPECTED_STATUS => ReadinessCheck::Ready,
        Ok(_) => ReadinessCheck::Rejected,
        Err(_) => ReadinessCheck::NotReady,
    }
}

fn terminate_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id().to_string();

        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{pid}")])
            .status();

        thread::sleep(Duration::from_millis(100));

        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .status();
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}
