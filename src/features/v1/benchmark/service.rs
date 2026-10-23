use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::config::env::Env;

use super::{
    discovery,
    error::BenchmarkError,
    live::{Metric, RunContext, RunPermit, Stage},
    metrics::{artifact_size, build_time},
    model::{BenchmarkRequest, BenchmarkResult, Metrics, Target},
    runtime, validation,
};

// Status reported for a step that has no status of its own
const STATUS_SUCCESS: &str = "success";

// Validation and target preparation. Nothing is started and no execution slot is
// needed, so an invalid request is rejected the same way whether or not a run is active.
pub fn prepare(request: &BenchmarkRequest) -> Result<PathBuf, BenchmarkError> {
    validation::validate(request).map_err(BenchmarkError::Validation)?;

    prepare_target(&request.path)
}

// The original entry point, unchanged. It does not take part in the execution slot, so
// the HTTP handlers do not use it: they call `execute` with a permit instead.
pub fn run(request: &BenchmarkRequest, env: &Env) -> Result<BenchmarkResult, BenchmarkError> {
    let target = prepare(request)?;

    run_core(&target, request, env, Uuid::now_v7(), &RunContext::noop())
}

// Runs a prepared request. The permit proves the caller owns the execution slot.
pub fn execute(
    target: &Path,
    request: &BenchmarkRequest,
    env: &Env,
    run_id: Uuid,
    ctx: &RunContext,
    _permit: &RunPermit,
) -> Result<BenchmarkResult, BenchmarkError> {
    run_core(target, request, env, run_id, ctx)
}

fn run_core(
    target: &Path,
    request: &BenchmarkRequest,
    env: &Env,
    run_id: Uuid,
    ctx: &RunContext,
) -> Result<BenchmarkResult, BenchmarkError> {
    // A cancel request is looked at before every step and inside each long one
    ctx.check_cancelled()?;

    ctx.stage_started(Stage::Discovery);
    let discovery = discovery::discover(target)?;
    ctx.measurement(Metric::Discovery, &discovery);
    ctx.stage_completed(Stage::Discovery, STATUS_SUCCESS, None);

    ctx.check_cancelled()?;

    ctx.stage_started(Stage::ArtifactSize);
    let artifact_size = artifact_size::measure(target)?;
    ctx.measurement(Metric::ArtifactSize, &artifact_size);
    ctx.stage_completed(Stage::ArtifactSize, STATUS_SUCCESS, None);

    ctx.check_cancelled()?;

    ctx.stage_started(Stage::Build);
    let build_time =
        build_time::measure_cancellable(target, discovery.runtime.as_deref(), ctx.control())?;
    ctx.measurement(Metric::BuildTime, &build_time);
    ctx.stage_completed(Stage::Build, &build_time.status, build_time.duration_ms);

    ctx.check_cancelled()?;

    let runtime_metrics =
        runtime::run_observed(target, discovery.runtime.as_deref(), request, env, ctx)?;

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
