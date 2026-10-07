use std::{path::Path, time::Duration};

use crate::config::env::Env;

use super::{
    metrics::{
        memory::MemoryConfig,
        startup_time::{self, StartupPlan, StartupPreparation},
    },
    model::{BenchmarkRequest, Cpu, HttpLatency, Memory, StartupTime},
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
    run_with_config(target, runtime, request, env, &RuntimeConfig::default())
}

// Owns the target process: start, readiness, startup, workload, memory, CPU, shutdown
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

    // Workload, memory and CPU are only meaningful for a ready target
    if startup.status != "success" {
        return Ok(RuntimeMetrics::startup_only(startup));
    }

    let Some(workload) = request.workload.as_ref() else {
        return Ok(RuntimeMetrics::startup_only(startup));
    };

    let workload_plan = WorkloadPlan::new(workload, &host, port, config.request_timeout)?;

    let phase = workload_phase::run(process, &workload_plan, &config.memory)?;

    Ok(RuntimeMetrics {
        startup_time: startup,
        memory: phase.memory,
        cpu: phase.cpu,
        http_latency: Some(phase.http_latency),
    })
}
