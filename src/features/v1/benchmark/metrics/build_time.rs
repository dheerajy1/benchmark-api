use std::{
    fs,
    path::Path,
    process::{Command, ExitStatus},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use crate::features::v1::benchmark::model::{BuildCommand, BuildTime};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

struct BuildTimeConfig {
    timeout: Duration,
}

impl Default for BuildTimeConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

pub fn measure(target: &Path, runtime: Option<&str>) -> Result<BuildTime, String> {
    measure_with_config(target, runtime, &BuildTimeConfig::default())
}

pub fn measure_with_timeout(
    target: &Path,
    runtime: Option<&str>,
    timeout: Duration,
) -> Result<BuildTime, String> {
    measure_with_config(target, runtime, &BuildTimeConfig { timeout })
}

fn measure_with_config(
    target: &Path,
    runtime: Option<&str>,
    config: &BuildTimeConfig,
) -> Result<BuildTime, String> {
    let command = discover_build_command(target, runtime)?;

    let Some(command) = command else {
        return Ok(BuildTime {
            status: "no_build_command".to_string(),
            duration_ms: None,
            command: None,
            working_directory: None,
            started_at: None,
            finished_at: None,
            exit_code: None,
        });
    };

    let working_directory = target
        .canonicalize()
        .map_err(|error| format!("Failed to resolve working directory: {error}"))?;

    let started_at = format_timestamp(SystemTime::now());
    let started = Instant::now();

    let mut child = spawn_build_process(&command, &working_directory)?;

    let outcome = wait_for_process(&mut child, config.timeout);

    let finished_at = format_timestamp(SystemTime::now());
    let duration_ms = started.elapsed().as_millis() as u64;

    match outcome {
        ProcessOutcome::Completed(status) => Ok(BuildTime {
            status: if status.success() {
                "success".to_string()
            } else {
                "failed".to_string()
            },
            duration_ms: Some(duration_ms),
            command: Some(command),
            working_directory: Some(working_directory.display().to_string()),
            started_at: Some(started_at),
            finished_at: Some(finished_at),
            exit_code: status.code(),
        }),
        ProcessOutcome::TimedOut => Ok(BuildTime {
            status: "timeout".to_string(),
            duration_ms: Some(duration_ms),
            command: Some(command),
            working_directory: Some(working_directory.display().to_string()),
            started_at: Some(started_at),
            finished_at: Some(finished_at),
            exit_code: None,
        }),
    }
}

pub fn discover_build_command(
    target: &Path,
    runtime: Option<&str>,
) -> Result<Option<BuildCommand>, String> {
    let program = match runtime {
        Some("bun") => "bun",
        Some("node") => "npm",
        Some("python" | "rust" | "go") | None => return Ok(None),
        Some(other) => {
            return Err(format!("Unsupported runtime for build_time: {other}"));
        }
    };

    let package_json_path = target.join("package.json");

    let contents = fs::read_to_string(&package_json_path)
        .map_err(|error| format!("Failed to read {}: {error}", package_json_path.display()))?;

    let package_json: Value = serde_json::from_str(&contents)
        .map_err(|error| format!("Failed to parse {}: {error}", package_json_path.display()))?;

    let Some(build_script) = package_json
        .get("scripts")
        .and_then(Value::as_object)
        .and_then(|scripts| scripts.get("build"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };

    if build_script.is_empty() {
        return Ok(None);
    }

    Ok(Some(BuildCommand {
        program: program.to_string(),
        args: vec!["run".to_string(), "build".to_string()],
    }))
}

fn spawn_build_process(
    command: &BuildCommand,
    working_directory: &Path,
) -> Result<std::process::Child, String> {
    #[cfg(unix)]
    {
        let mut process = Command::new("setsid");
        process
            .arg(&command.program)
            .args(&command.args)
            .current_dir(working_directory);

        process
            .spawn()
            .map_err(|error| format!("Failed to spawn build command: {error}"))
    }

    #[cfg(not(unix))]
    {
        Command::new(&command.program)
            .args(&command.args)
            .current_dir(working_directory)
            .spawn()
            .map_err(|error| format!("Failed to spawn build command: {error}"))
    }
}

enum ProcessOutcome {
    Completed(ExitStatus),
    TimedOut,
}

fn wait_for_process(child: &mut std::process::Child, timeout: Duration) -> ProcessOutcome {
    let started = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return ProcessOutcome::Completed(status),
            Ok(None) => {}
            Err(_) => {
                terminate_process_tree(child);
                let _ = child.wait();
                return ProcessOutcome::TimedOut;
            }
        }

        if started.elapsed() >= timeout {
            terminate_process_tree(child);
            let _ = child.wait();
            return ProcessOutcome::TimedOut;
        }

        thread::sleep(Duration::from_millis(20));
    }
}

fn terminate_process_tree(child: &mut std::process::Child) {
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

fn format_timestamp(time: SystemTime) -> String {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));

    let total_seconds = duration.as_secs();
    let milliseconds = duration.subsec_millis();

    let days = total_seconds / 86_400;
    let seconds_of_day = total_seconds % 86_400;

    let (year, month, day) = civil_from_days(days as i64);

    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milliseconds:03}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };

    (year, month, day)
}
