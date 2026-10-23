use std::{path::Path, time::Duration};

use crate::config::env::Env;

use super::{
    live::{Metric, RunContext, Stage, skip},
    metrics::{
        memory::MemoryConfig,
        startup_time::{self, StartupPlan, StartupPreparation},
    },
    model::{BenchmarkRequest, Cpu, HttpLatency, Memory, StartupTime},
    stop::Stop,
    target_process::TargetProcess,
    workload::plan::{DEFAULT_REQUEST_TIMEOUT, WorkloadPlan},
    workload_phase,
};

pub struct RuntimeConfig {
    pub memory: MemoryConfig,
    pub request_timeout: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            memory: MemoryConfig::default(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
}

pub struct RuntimeMetrics {
    pub startup_time: StartupTime,
    pub memory: Option<Memory>,
    pub cpu: Option<Cpu>,
    pub http_latency: Option<HttpLatency>,
}

impl RuntimeMetrics {
    fn startup_only(startup_time: StartupTime) -> Self {
        Self {
            startup_time,
            memory: None,
            cpu: None,
            http_latency: None,
        }
    }
}

pub fn run(
    target: &Path,
    runtime: Option<&str>,
    request: &BenchmarkRequest,
    env: &Env,
) -> Result<RuntimeMetrics, String> {
    run_observed(target, runtime, request, env, &RunContext::noop()).map_err(Stop::into_message)
}

// Same run, reporting its progress to the context and stopping early on a cancel request
pub fn run_observed(
    target: &Path,
    runtime: Option<&str>,
    request: &BenchmarkRequest,
    env: &Env,
    ctx: &RunContext,
) -> Result<RuntimeMetrics, Stop> {
    run_with_config_observed(
        target,
        runtime,
        request,
        env,
        &RuntimeConfig::default(),
        ctx,
    )
}

// Owns the target process: start, readiness, startup, workload, memory, CPU, shutdown
pub fn run_with_config(
    target: &Path,
    runtime: Option<&str>,
    request: &BenchmarkRequest,
    env: &Env,
    config: &RuntimeConfig,
) -> Result<RuntimeMetrics, String> {
    run_with_config_observed(target, runtime, request, env, config, &RunContext::noop())
        .map_err(Stop::into_message)
}

pub fn run_with_config_observed(
    target: &Path,
    runtime: Option<&str>,
    request: &BenchmarkRequest,
    env: &Env,
    config: &RuntimeConfig,
    ctx: &RunContext,
) -> Result<RuntimeMetrics, Stop> {
    let plan = match startup_time::prepare_with_startup(target, runtime, request.startup.as_ref())?
    {
        StartupPreparation::Skipped(result) => {
            ctx.stage_started(Stage::Startup);
            ctx.measurement(Metric::StartupTime, &result);
            ctx.stage_completed(Stage::Startup, &result.status, result.duration_ms);

            let reason = if request.workload.is_some() {
                skip::NO_START_COMMAND
            } else {
                skip::NO_WORKLOAD
            };

            ctx.stage_skipped(Stage::Workload, reason);

            return Ok(RuntimeMetrics::startup_only(result));
        }
        StartupPreparation::Launch(plan) => plan,
    };

    if ctx.control().is_cancel_requested() {
        return Err(Stop::Cancelled);
    }

    let mut process = TargetProcess::start(&plan.command, &plan.working_directory)?;

    let result = run_phases(&mut process, plan, request, env, config, ctx);

    // Shutdown happens exactly once, on success and on every error path, and the whole process
    // group is checked to be gone
    let shutdown = process.shutdown_verified();

    settle(result, shutdown)
}

// Combines how the phases ended with how the target was stopped. A cancelled run is only
// reported as cancelled when the target is really gone. For any other ending a target that
// could not be stopped completely is reported on stderr and the result stands.
pub fn settle(
    result: Result<RuntimeMetrics, Stop>,
    shutdown: Result<(), String>,
) -> Result<RuntimeMetrics, Stop> {
    match (result, shutdown) {
        (Err(Stop::Cancelled), Err(message)) => Err(Stop::Failed(format!(
            "The run was cancelled but the target could not be stopped completely: {message}"
        ))),
        (result, Err(message)) => {
            eprintln!("warning: the target could not be stopped completely, {message}");

            result
        }
        (result, Ok(())) => result,
    }
}

fn run_phases(
    process: &mut TargetProcess,
    plan: StartupPlan,
    request: &BenchmarkRequest,
    env: &Env,
    config: &RuntimeConfig,
    ctx: &RunContext,
) -> Result<RuntimeMetrics, Stop> {
    let host = plan.readiness.host.clone();
    let port = plan.readiness.port;

    ctx.stage_started(Stage::Startup);
    let startup = startup_time::observe_cancellable(
        plan,
        process,
        env,
        startup_time::DEFAULT_TIMEOUT,
        ctx.control(),
    )?;
    ctx.measurement(Metric::StartupTime, &startup);
    ctx.stage_completed(Stage::Startup, &startup.status, startup.duration_ms);

    // Workload, memory and CPU are only meaningful for a ready target
    if startup.status != "success" {
        ctx.stage_skipped(Stage::Workload, skip::STARTUP_NOT_SUCCESSFUL);

        return Ok(RuntimeMetrics::startup_only(startup));
    }

    let Some(workload) = request.workload.as_ref() else {
        ctx.stage_skipped(Stage::Workload, skip::NO_WORKLOAD);

        return Ok(RuntimeMetrics::startup_only(startup));
    };

    let workload_plan = WorkloadPlan::new(workload, &host, port, config.request_timeout)?;

    if ctx.control().is_cancel_requested() {
        return Err(Stop::Cancelled);
    }

    let phase = workload_phase::run_observed(process, &workload_plan, &config.memory, ctx)?;

    Ok(RuntimeMetrics {
        startup_time: startup,
        memory: phase.memory,
        cpu: phase.cpu,
        http_latency: Some(phase.http_latency),
    })
}
