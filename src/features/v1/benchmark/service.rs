use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use uuid::Uuid;

use crate::config::env::Env;

use super::{
    discovery,
    docker::DockerCli,
    error::BenchmarkError,
    live::{Metric, RunContext, RunPermit, Stage},
    metrics::{artifact_size, build_time, memory::MemoryConfig},
    model::{BenchmarkRequest, BenchmarkResult},
    runtime::{self, RuntimeProgress},
    validation,
    workspace::{
        orchestrator,
        result::{STATUS_SUCCESS as WORKSPACE_SUCCESS, WorkspaceResult},
    },
};

// Status reported for a step that has no status of its own
const STATUS_SUCCESS: &str = "success";

// Validation and target preparation. Nothing is started and no execution slot is
// needed, so an invalid request is rejected the same way whether or not a run is active.
pub fn prepare(request: &BenchmarkRequest) -> Result<PathBuf, BenchmarkError> {
    validation::validate(request).map_err(BenchmarkError::Validation)?;

    let target = prepare_target(&request.path)?;

    // A workspace that is already running is refused before anything starts
    orchestrator::precheck(&target, &request.workspaces, &DockerCli)
        .map_err(BenchmarkError::Conflict)?;

    Ok(target)
}

// How a run ended, with the work it had finished by then
pub struct RunReport {
    // Every part that finished is filled, the parts of a step that did not are None
    pub result: BenchmarkResult,
    pub end: RunEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEnd {
    Completed,
    // Stopped on request
    Cancelled,
    // Failed on its own
    Failed(BenchmarkError),
}

impl RunReport {
    // The all or nothing view: the result of a completed run, otherwise the error
    pub fn into_result(self) -> Result<BenchmarkResult, BenchmarkError> {
        match self.end {
            RunEnd::Completed => Ok(self.result),
            RunEnd::Cancelled => Err(BenchmarkError::Cancelled),
            RunEnd::Failed(error) => Err(error),
        }
    }
}

// Runs one request without the execution slot and without progress reporting
pub fn run(request: &BenchmarkRequest, env: &Env) -> Result<BenchmarkResult, BenchmarkError> {
    let target = prepare(request)?;

    run_report(&target, request, env, Uuid::now_v7(), &RunContext::noop()).into_result()
}

// Runs a prepared request. The permit proves the caller owns the execution slot.
pub fn execute(
    target: &Path,
    request: &BenchmarkRequest,
    env: &Env,
    run_id: Uuid,
    ctx: &RunContext,
    permit: &RunPermit,
) -> Result<BenchmarkResult, BenchmarkError> {
    execute_report(target, request, env, run_id, ctx, permit).into_result()
}

// Same run, keeping what finished when the run fails or is cancelled
pub fn execute_report(
    target: &Path,
    request: &BenchmarkRequest,
    env: &Env,
    run_id: Uuid,
    ctx: &RunContext,
    _permit: &RunPermit,
) -> RunReport {
    run_report(target, request, env, run_id, ctx)
}

fn run_report(
    target: &Path,
    request: &BenchmarkRequest,
    env: &Env,
    run_id: Uuid,
    ctx: &RunContext,
) -> RunReport {
    let mut result = BenchmarkResult::empty(run_id, target.display().to_string());

    let end = match fill(target, request, env, ctx, &mut result) {
        Ok(()) => RunEnd::Completed,
        Err(BenchmarkError::Cancelled) => RunEnd::Cancelled,
        Err(error) => RunEnd::Failed(error),
    };

    RunReport { result, end }
}

// Runs the steps in order and puts each result into `result` as soon as its step is complete
fn fill(
    target: &Path,
    request: &BenchmarkRequest,
    env: &Env,
    ctx: &RunContext,
    result: &mut BenchmarkResult,
) -> Result<(), BenchmarkError> {
    // A cancel request is looked at before every step and inside each long one
    ctx.check_cancelled()?;

    ctx.stage_started(Stage::Discovery);
    let discovery = discovery::discover(target)?;
    ctx.measurement(Metric::Discovery, &discovery);
    ctx.stage_completed(Stage::Discovery, STATUS_SUCCESS, None);
    result.discovery = discovery;

    let runtime_code = result.discovery.runtime.clone();

    ctx.check_cancelled()?;

    ctx.stage_started(Stage::ArtifactSize);
    let artifact_size = artifact_size::measure(target)?;
    ctx.measurement(Metric::ArtifactSize, &artifact_size);
    ctx.stage_completed(Stage::ArtifactSize, STATUS_SUCCESS, None);
    result.metrics.artifact_size = Some(artifact_size);

    ctx.check_cancelled()?;

    ctx.stage_started(Stage::Build);
    let build_time =
        build_time::measure_cancellable(target, runtime_code.as_deref(), ctx.control())?;
    ctx.measurement(Metric::BuildTime, &build_time);
    ctx.stage_completed(Stage::Build, &build_time.status, build_time.duration_ms);
    result.metrics.build_time = Some(build_time);

    ctx.check_cancelled()?;

    let mut progress = RuntimeProgress::default();

    let outcome = runtime::run_observed_into(
        target,
        runtime_code.as_deref(),
        request,
        env,
        ctx,
        &mut progress,
    );

    // What finished is kept, whether the steps ended well or not
    progress.apply(&mut result.metrics);
    outcome?;

    // The main project has finished and shut down, the workspaces follow one at a time
    run_workspaces(target, request, ctx, &mut result.metrics.workspaces)?;

    Ok(())
}

fn run_workspaces(
    target: &Path,
    request: &BenchmarkRequest,
    ctx: &RunContext,
    results: &mut Vec<WorkspaceResult>,
) -> Result<(), BenchmarkError> {
    if request.workspaces.is_empty() {
        return Ok(());
    }

    ctx.check_cancelled()?;
    ctx.stage_started(Stage::Workspaces);

    let started = Instant::now();

    orchestrator::run_all_into(
        target,
        &request.workspaces,
        &DockerCli,
        &MemoryConfig::default(),
        ctx,
        results,
    )?;

    let status = if results
        .iter()
        .all(|result| result.status == WORKSPACE_SUCCESS)
    {
        STATUS_SUCCESS
    } else {
        "failed"
    };

    ctx.stage_completed(
        Stage::Workspaces,
        status,
        u64::try_from(started.elapsed().as_millis()).ok(),
    );

    Ok(())
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
