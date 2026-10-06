use std::{path::Path, time::Duration};

use crate::config::env::Env;

use super::{
    metrics::{
        http_latency,
        memory::{self, MemoryConfig},
        startup_time::{self, StartupPlan, StartupPreparation},
    },
    model::{BenchmarkRequest, HttpLatency, Memory, StartupTime},
    target_process::TargetProcess,
    workload::{
        executor,
        plan::{DEFAULT_REQUEST_TIMEOUT, WorkloadPlan},
    },
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
    pub http_latency: Option<HttpLatency>,
}

impl RuntimeMetrics {
    fn startup_only(startup_time: StartupTime) -> Self {
        Self {
            startup_time,
            memory: None,
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
    run_with_config(target, runtime, request, env, &RuntimeConfig::default())
}

// Owns the target process: start, readiness, startup, workload, memory, shutdown
pub fn run_with_config(
    target: &Path,
    runtime: Option<&str>,
    request: &BenchmarkRequest,
    env: &Env,
    config: &RuntimeConfig,
) -> Result<RuntimeMetrics, String> {
    let plan = match startup_time::prepare_with_startup(target, runtime, request.startup.as_ref())?
    {
        StartupPreparation::Skipped(result) => return Ok(RuntimeMetrics::startup_only(result)),
        StartupPreparation::Launch(plan) => plan,
    };

    let mut process = TargetProcess::start(&plan.command, &plan.working_directory)?;

    let result = run_phases(&mut process, plan, request, env, config);

    // Shutdown happens exactly once, on success and on every error path
    process.shutdown();

    result
}

fn run_phases(
    process: &mut TargetProcess,
    plan: StartupPlan,
    request: &BenchmarkRequest,
    env: &Env,
    config: &RuntimeConfig,
) -> Result<RuntimeMetrics, String> {
    let host = plan.readiness.host.clone();
    let port = plan.readiness.port;

    let startup = startup_time::observe(plan, process, env, startup_time::DEFAULT_TIMEOUT)?;

    // Workload and memory are only meaningful for a ready target
    if startup.status != "success" {
        return Ok(RuntimeMetrics::startup_only(startup));
    }

    let Some(workload) = request.workload.as_ref() else {
        return Ok(RuntimeMetrics::startup_only(startup));
    };

    let workload_plan = WorkloadPlan::new(workload, &host, port, config.request_timeout)?;

    let (memory, http_latency) = run_workload_phase(process, &workload_plan, &config.memory)?;

    Ok(RuntimeMetrics {
        startup_time: startup,
        memory,
        http_latency: Some(http_latency),
    })
}

// Baseline, continuous RSS sampling during the workload, then finalized peak and delta
fn run_workload_phase(
    process: &mut TargetProcess,
    plan: &WorkloadPlan,
    memory_config: &MemoryConfig,
) -> Result<(Option<Memory>, HttpLatency), String> {
    let session = match memory::begin_for_group(memory_config, process.process_group_id()) {
        Ok(session) => session,
        Err(error) => {
            if process.is_running() {
                return Err(error);
            }

            // Target disappeared before the workload could start
            let latency = http_latency::summarize(&[], Duration::ZERO, true);

            return Ok((None, latency));
        }
    };

    let outcome = executor::execute(plan, || process.is_running());

    // Sampler is always stopped, even when the workload failed
    let finished = session.finish();

    let outcome = outcome?;
    let finished = finished?;

    let latency = http_latency::summarize(
        &outcome.observations,
        outcome.elapsed,
        outcome.target_exited,
    );

    Ok((Some(finished.memory), latency))
}
