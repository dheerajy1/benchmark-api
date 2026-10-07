use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::config::env::Env;

use super::{
    discovery,
    error::BenchmarkError,
    metrics::{artifact_size, build_time},
    model::{BenchmarkRequest, BenchmarkResult, Metrics, Target},
    runtime, validation,
};

pub fn run(request: &BenchmarkRequest, env: &Env) -> Result<BenchmarkResult, BenchmarkError> {
    validation::validate(request).map_err(BenchmarkError::Validation)?;

    let target = prepare_target(&request.path)?;
    let run_id = Uuid::now_v7();

    let discovery = discovery::discover(&target)?;
    let artifact_size = artifact_size::measure(&target)?;
    let build_time = build_time::measure(&target, discovery.runtime.as_deref())?;

    let runtime_metrics = runtime::run(&target, discovery.runtime.as_deref(), request, env)?;

    Ok(BenchmarkResult {
        run_id,
        target: Target {
            path: target.display().to_string(),
        },
        discovery,
        metrics: Metrics {
            artifact_size,
            build_time,
            startup_time: runtime_metrics.startup_time,
            memory: runtime_metrics.memory,
            cpu: runtime_metrics.cpu,
            http_latency: runtime_metrics.http_latency,
        },
    })
}

fn prepare_target(path: &Path) -> Result<PathBuf, BenchmarkError> {
    let prepared = if path.starts_with("~") {
        let home = std::env::var("HOME").map_err(|_| {
            BenchmarkError::Internal("HOME environment variable is not set".to_string())
        })?;

        let suffix = path.strip_prefix("~").unwrap_or(path);
        Path::new(&home).join(suffix)
    } else {
        path.to_path_buf()
    };

    if !prepared.exists() {
        return Err(BenchmarkError::InvalidPath(format!(
            "Path does not exist: {}",
            prepared.display()
        )));
    }

    if !prepared.is_dir() {
        return Err(BenchmarkError::InvalidPath(format!(
            "Path is not a directory: {}",
            prepared.display()
        )));
    }

    Ok(prepared)
}
