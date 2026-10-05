use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use crate::config::env::Env;
use crate::features::v1::benchmark::{
    model::{BuildCommand, StartupReadiness, StartupTime},
    target_process::{
        EXPECTED_STATUS, READINESS_HOST, READINESS_METHOD, READINESS_PATH, ReadyOutcome,
        TargetProcess,
    },
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

// Everything needed to launch the target and describe its startup
pub struct StartupPlan {
    pub command: BuildCommand,
    pub port: u16,
    pub working_directory: PathBuf,
}

pub enum StartupPreparation {
    // Startup cannot be measured, the result is already final
    Skipped(StartupTime),
    // Target can be launched using this plan
    Launch(StartupPlan),
}

pub fn measure(target: &Path, runtime: Option<&str>, env: &Env) -> Result<StartupTime, String> {
    measure_with_timeout(target, runtime, env, DEFAULT_TIMEOUT)
}

// Convenience wrapper that owns the full lifecycle: start, observe, shutdown
pub fn measure_with_timeout(
    target: &Path,
    runtime: Option<&str>,
    env: &Env,
    timeout: Duration,
) -> Result<StartupTime, String> {
    let plan = match prepare(target, runtime)? {
        StartupPreparation::Skipped(result) => return Ok(result),
        StartupPreparation::Launch(plan) => plan,
    };

    let mut process = TargetProcess::start(&plan.command, &plan.working_directory)?;

    let result = observe(plan, &mut process, env, timeout);

    process.shutdown();

    result
}

pub fn prepare(target: &Path, runtime: Option<&str>) -> Result<StartupPreparation, String> {
    let Some(command) = discover_start_command(target, runtime)? else {
        return Ok(StartupPreparation::Skipped(no_start_command()));
    };

    let Some(port) = discover_port(target, &command)? else {
        return Ok(StartupPreparation::Skipped(StartupTime {
            status: "no_port".to_string(),
            duration_ms: None,
            command: Some(command),
            working_directory: None,
            readiness: None,
            started_at: None,
            ready_at: None,
            exit_code: None,
        }));
    };

    let working_directory = target
        .canonicalize()
        .map_err(|error| format!("Failed to resolve working directory: {error}"))?;

    Ok(StartupPreparation::Launch(StartupPlan {
        command,
        port,
        working_directory,
    }))
}

// Observes the startup of a running target, never shuts the process down
pub fn observe(
    plan: StartupPlan,
    process: &mut TargetProcess,
    env: &Env,
    timeout: Duration,
) -> Result<StartupTime, String> {
    let outcome = process.wait_until_ready(plan.port, env, timeout)?;

    let duration_ms = process.startup_elapsed().as_millis() as u64;
    let started_at = format_timestamp(process.started_at());

    let readiness = StartupReadiness {
        readiness_type: "http".to_string(),
        host: READINESS_HOST.to_string(),
        port: plan.port,
        path: READINESS_PATH.to_string(),
        method: READINESS_METHOD.to_string(),
        expected_status: EXPECTED_STATUS,
    };

    let working_directory = plan.working_directory.display().to_string();

    let (status, ready_at, exit_code) = match outcome {
        ReadyOutcome::Ready => ("success", Some(format_timestamp(SystemTime::now())), None),
        ReadyOutcome::Rejected => ("unhealthy", None, None),
        ReadyOutcome::Exited(code) => ("failed", None, code),
        ReadyOutcome::TimedOut => ("timeout", None, None),
    };

    Ok(StartupTime {
        status: status.to_string(),
        duration_ms: Some(duration_ms),
        command: Some(plan.command),
        working_directory: Some(working_directory),
        readiness: Some(readiness),
        started_at: Some(started_at),
        ready_at,
        exit_code,
    })
}

fn no_start_command() -> StartupTime {
    StartupTime {
        status: "no_start_command".to_string(),
        duration_ms: None,
        command: None,
        working_directory: None,
        readiness: None,
        started_at: None,
        ready_at: None,
        exit_code: None,
    }
}

pub fn discover_start_command(
    target: &Path,
    runtime: Option<&str>,
) -> Result<Option<BuildCommand>, String> {
    match runtime {
        Some("bun") => discover_package_start_command(target, "bun"),
        Some("node") => discover_package_start_command(target, "npm"),
        Some("rust") => Ok(Some(BuildCommand {
            program: "cargo".to_string(),
            args: vec!["run".to_string()],
        })),
        Some("python") => Ok(None),
        Some("go") => Ok(None),
        None => Ok(None),
        Some(other) => Err(format!("Unsupported runtime for startup_time: {other}")),
    }
}

fn discover_package_start_command(
    target: &Path,
    program: &str,
) -> Result<Option<BuildCommand>, String> {
    let package_json_path = target.join("package.json");

    let contents = fs::read_to_string(&package_json_path)
        .map_err(|error| format!("Failed to read {}: {error}", package_json_path.display()))?;

    let package_json: Value = serde_json::from_str(&contents)
        .map_err(|error| format!("Failed to parse {}: {error}", package_json_path.display()))?;

    let Some(start_script) = package_json
        .get("scripts")
        .and_then(Value::as_object)
        .and_then(|scripts| scripts.get("start"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };

    if start_script.is_empty() {
        return Ok(None);
    }

    Ok(Some(BuildCommand {
        program: program.to_string(),
        args: vec!["run".to_string(), "start".to_string()],
    }))
}

fn discover_port(target: &Path, command: &BuildCommand) -> Result<Option<u16>, String> {
    let Some(env_file) = discover_env_file(target, command) else {
        return Ok(None);
    };

    let contents = fs::read_to_string(&env_file)
        .map_err(|error| format!("Failed to read {}: {error}", env_file.display()))?;

    for line in contents.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        if key.trim() != "APP_PORT" {
            continue;
        }

        let value = value.trim().trim_matches('"').trim_matches('\'');

        let port = value
            .parse::<u16>()
            .map_err(|error| format!("Invalid PORT value '{value}': {error}"))?;

        return Ok(Some(port));
    }

    Ok(None)
}

fn discover_env_file(target: &Path, command: &BuildCommand) -> Option<PathBuf> {
    if command.program == "bun" || command.program == "npm" {
        let package_json_path = target.join("package.json");

        if let Ok(contents) = fs::read_to_string(package_json_path) {
            if let Ok(package_json) = serde_json::from_str::<Value>(&contents) {
                if let Some(start_script) = package_json
                    .get("scripts")
                    .and_then(Value::as_object)
                    .and_then(|scripts| scripts.get("start"))
                    .and_then(Value::as_str)
                {
                    let tokens: Vec<&str> = start_script.split_whitespace().collect();

                    for index in 0..tokens.len() {
                        if tokens[index] == "--env-file" {
                            if let Some(path) = tokens.get(index + 1) {
                                return Some(target.join(path));
                            }
                        }

                        if let Some(path) = tokens[index].strip_prefix("--env-file=") {
                            return Some(target.join(path));
                        }
                    }
                }
            }
        }
    }

    for filename in [".env.production", ".env", ".env.local"] {
        let path = target.join(filename);

        if path.is_file() {
            return Some(path);
        }
    }

    None
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
