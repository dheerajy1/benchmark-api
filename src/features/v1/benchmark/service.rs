use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::config::env::Env;

use super::{
    discovery,
    metrics::{artifact_size, build_time, memory, startup_time},
    model::{BenchmarkRequest, BenchmarkResult, Memory, Metrics, StartupTime, Target},
    target_process::TargetProcess,
};

pub fn run(request: &BenchmarkRequest, env: &Env) -> Result<BenchmarkResult, String> {
    let target = prepare_target(&request.path)?;
    let run_id = Uuid::now_v7();

    let discovery = discovery::discover(&target)?;
    let artifact_size = artifact_size::measure(&target)?;
    let build_time = build_time::measure(&target, discovery.runtime.as_deref())?;

    let (startup_time, memory) = run_target_metrics(&target, discovery.runtime.as_deref(), env)?;

    Ok(BenchmarkResult {
        run_id,
        target: Target {
            path: target.display().to_string(),
        },
        discovery,
        metrics: Metrics {
            artifact_size,
            build_time,
            startup_time,
            memory,
        },
    })
}

// Owns the target process: start, readiness, startup, memory, shutdown
fn run_target_metrics(
    target: &Path,
    runtime: Option<&str>,
    env: &Env,
) -> Result<(StartupTime, Option<Memory>), String> {
    let plan = match startup_time::prepare(target, runtime)? {
        startup_time::StartupPreparation::Skipped(result) => return Ok((result, None)),
        startup_time::StartupPreparation::Launch(plan) => plan,
    };

    let mut process = TargetProcess::start(&plan.command, &plan.working_directory)?;

    let startup =
        match startup_time::observe(plan, &mut process, env, startup_time::DEFAULT_TIMEOUT) {
            Ok(startup) => startup,
            Err(error) => {
                process.shutdown();
                return Err(error);
            }
        };

    // Memory is only measured when the target reached the ready state
    let memory = if startup.status == "success" {
        match memory::measure(&mut process) {
            Ok(memory) => Some(memory),
            Err(error) => {
                process.shutdown();
                return Err(error);
            }
        }
    } else {
        None
    };

    process.shutdown();

    Ok((startup, memory))
}

fn prepare_target(path: &Path) -> Result<PathBuf, String> {
    let prepared = if path.starts_with("~") {
        let home = std::env::var("HOME")
            .map_err(|_| "HOME environment variable is not set".to_string())?;

        let suffix = path.strip_prefix("~").unwrap_or(path);
        Path::new(&home).join(suffix)
    } else {
        path.to_path_buf()
    };

    if !prepared.exists() {
        return Err(format!("Path does not exist: {}", prepared.display()));
    }

    if !prepared.is_dir() {
        return Err(format!("Path is not a directory: {}", prepared.display()));
    }

    Ok(prepared)
}
