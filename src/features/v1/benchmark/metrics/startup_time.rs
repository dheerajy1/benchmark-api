use std::{
    fs,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use crate::features::v1::benchmark::model::{BuildCommand, StartupReadiness, StartupTime};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const READINESS_PATH: &str = "/api/v1/health/app";
const READINESS_HOST: &str = "127.0.0.1";
const READINESS_METHOD: &str = "GET";
const EXPECTED_STATUS: u16 = 200;

struct StartupTimeConfig {
    timeout: Duration,
}

impl Default for StartupTimeConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

pub fn measure(
    target: &Path,
    runtime: Option<&str>,
    env: &crate::config::env::Env,
) -> Result<StartupTime, String> {
    measure_with_config(target, runtime, env, &StartupTimeConfig::default())
}

pub fn measure_with_timeout(
    target: &Path,
    runtime: Option<&str>,
    env: &crate::config::env::Env,
    timeout: Duration,
) -> Result<StartupTime, String> {
    measure_with_config(target, runtime, env, &StartupTimeConfig { timeout })
}

fn measure_with_config(
    target: &Path,
    runtime: Option<&str>,
    env: &crate::config::env::Env,
    config: &StartupTimeConfig,
) -> Result<StartupTime, String> {
    let Some(command) = discover_start_command(target, runtime)? else {
        return Ok(no_start_command());
    };

    let Some(port) = discover_port(target, &command)? else {
        return Ok(StartupTime {
            status: "no_port".to_string(),
            duration_ms: None,
            command: Some(command),
            working_directory: None,
            readiness: None,
            started_at: None,
            ready_at: None,
            exit_code: None,
        });
    };

    let working_directory = target
        .canonicalize()
        .map_err(|error| format!("Failed to resolve working directory: {error}"))?;

    let readiness = StartupReadiness {
        readiness_type: "http".to_string(),
        host: READINESS_HOST.to_string(),
        port,
        path: READINESS_PATH.to_string(),
        method: READINESS_METHOD.to_string(),
        expected_status: EXPECTED_STATUS,
    };

    let started_at = format_timestamp(SystemTime::now());
    let started = Instant::now();

    let mut child = spawn_start_process(&command, &working_directory)?;

    loop {
        match http_ready(READINESS_HOST, port, env) {
            ReadinessCheck::Ready => {
                let ready_at = format_timestamp(SystemTime::now());
                let duration_ms = started.elapsed().as_millis() as u64;

                terminate_process_tree(&mut child);
                let _ = child.wait();

                return Ok(StartupTime {
                    status: "success".to_string(),
                    duration_ms: Some(duration_ms),
                    command: Some(command),
                    working_directory: Some(working_directory.display().to_string()),
                    readiness: Some(readiness),
                    started_at: Some(started_at),
                    ready_at: Some(ready_at),
                    exit_code: None,
                });
            }
            ReadinessCheck::Rejected => {
                let duration_ms = started.elapsed().as_millis() as u64;

                terminate_process_tree(&mut child);
                let _ = child.wait();

                return Ok(StartupTime {
                    status: "unhealthy".to_string(),
                    duration_ms: Some(duration_ms),
                    command: Some(command),
                    working_directory: Some(working_directory.display().to_string()),
                    readiness: Some(readiness),
                    started_at: Some(started_at),
                    ready_at: None,
                    exit_code: None,
                });
            }
            ReadinessCheck::NotReady => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let duration_ms = started.elapsed().as_millis() as u64;

                return Ok(StartupTime {
                    status: "failed".to_string(),
                    duration_ms: Some(duration_ms),
                    command: Some(command),
                    working_directory: Some(working_directory.display().to_string()),
                    readiness: Some(readiness),
                    started_at: Some(started_at),
                    ready_at: None,
                    exit_code: status.code(),
                });
            }
            Ok(None) => {}
            Err(error) => {
                terminate_process_tree(&mut child);
                let _ = child.wait();

                return Err(format!("Failed to inspect startup process: {error}"));
            }
        }

        if started.elapsed() >= config.timeout {
            terminate_process_tree(&mut child);
            let _ = child.wait();

            let duration_ms = started.elapsed().as_millis() as u64;

            return Ok(StartupTime {
                status: "timeout".to_string(),
                duration_ms: Some(duration_ms),
                command: Some(command),
                working_directory: Some(working_directory.display().to_string()),
                readiness: Some(readiness),
                started_at: Some(started_at),
                ready_at: None,
                exit_code: None,
            });
        }

        thread::sleep(POLL_INTERVAL);
    }
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

fn discover_env_file(target: &Path, command: &BuildCommand) -> Option<std::path::PathBuf> {
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

fn spawn_start_process(
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

enum ReadinessCheck {
    // Server answered with the expected status
    Ready,
    // Server answered with any other status
    Rejected,
    // No usable answer yet, keep polling
    NotReady,
}

fn http_ready(host: &str, port: u16, env: &crate::config::env::Env) -> ReadinessCheck {
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
