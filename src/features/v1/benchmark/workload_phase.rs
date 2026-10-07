use std::{thread, time::Duration};

use super::{
    metrics::{
        cpu::{baseline, session},
        http_latency,
        memory::{self, MemoryConfig},
    },
    model::{Cpu, HttpLatency, Memory},
    target_process::TargetProcess,
    workload::{executor, plan::WorkloadPlan},
};

pub struct PhaseOutput {
    pub memory: Option<Memory>,
    pub cpu: Option<Cpu>,
    pub http_latency: HttpLatency,
}

// Stabilization, baselines, memory and CPU observers alongside the workload, then finalize
pub fn run(
    process: &mut TargetProcess,
    plan: &WorkloadPlan,
    config: &MemoryConfig,
) -> Result<PhaseOutput, String> {
    let group_id = process.process_group_id();

    // Stabilization is waited once and shared by both metrics
    thread::sleep(config.stabilization);

    let cpu_baseline = match baseline::measure_baseline_for_group(config.sample_interval, group_id)
    {
        Ok(value) => value,
        Err(error) => return unavailable_or_error(process, error),
    };

    // Stabilization already happened, so the memory baseline is read right away
    let memory_config = MemoryConfig {
        stabilization: Duration::ZERO,
        sample_interval: config.sample_interval,
    };

    let memory_session = match memory::begin_for_group(&memory_config, group_id) {
        Ok(started) => started,
        Err(error) => return unavailable_or_error(process, error),
    };

    let cpu_session = match session::begin_for_group(cpu_baseline, config.sample_interval, group_id)
    {
        Ok(started) => started,
        Err(error) => {
            // The memory sampler must not outlive this phase
            let _ = memory_session.finish();

            return unavailable_or_error(process, error);
        }
    };

    let outcome = executor::execute(plan, || process.is_running());

    // Observers are always stopped, even when the workload failed
    let cpu_finished = cpu_session.finish();
    let memory_finished = memory_session.finish();

    let outcome = outcome?;
    let cpu_finished = cpu_finished?;
    let memory_finished = memory_finished?;

    let latency = http_latency::summarize(
        &outcome.observations,
        outcome.elapsed,
        outcome.target_exited,
    );

    Ok(PhaseOutput {
        memory: Some(memory_finished.memory),
        cpu: cpu_finished.cpu,
        http_latency: latency,
    })
}

// A vanished target is an expected condition, a failure on a live target is an error
fn unavailable_or_error(process: &mut TargetProcess, error: String) -> Result<PhaseOutput, String> {
    if process.is_running() {
        return Err(error);
    }

    // Target disappeared before the workload could start
    let latency = http_latency::summarize(&[], Duration::ZERO, true);

    Ok(PhaseOutput {
        memory: None,
        cpu: None,
        http_latency: latency,
    })
}
