use std::{sync::Arc, time::Duration};

use super::{
    live::{LiveGauges, RunContext, SAMPLE_INTERVAL, Stage, gauges::build_sample, skip},
    metrics::{
        cpu::{baseline, session},
        http_latency,
        memory::{self, MemoryConfig},
    },
    model::{Cpu, HttpLatency, Memory},
    process_group,
    stop::{Stop, sleep_unless_cancelled},
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
    run_observed(process, plan, config, &RunContext::noop()).map_err(Stop::into_message)
}

// Same phase, reporting the baseline and workload stages to the context and stopping early on a
// cancel request. The caller shuts the target down and checks that it is gone.
pub fn run_observed(
    process: &mut TargetProcess,
    plan: &WorkloadPlan,
    config: &MemoryConfig,
    ctx: &RunContext,
) -> Result<PhaseOutput, Stop> {
    let group_id = process.process_group_id();

    ctx.stage_started(Stage::Baseline);

    // Stabilization is waited once and shared by both metrics
    if sleep_unless_cancelled(config.stabilization, ctx.control()) {
        return Err(Stop::Cancelled);
    }

    let cpu_baseline = match baseline::measure_baseline_for_group(config.sample_interval, group_id)
    {
        Ok(value) => value,
        Err(error) => return unavailable_or_error(process, error, ctx),
    };

    if ctx.control().is_cancel_requested() {
        return Err(Stop::Cancelled);
    }

    // Stabilization already happened, so the memory baseline is read right away
    let memory_config = MemoryConfig {
        stabilization: Duration::ZERO,
        sample_interval: config.sample_interval,
    };

    // Only a watched run measures live values, an unwatched one does no sampling work
    let gauges = ctx
        .samples_enabled()
        .then(|| Arc::new(LiveGauges::default()));

    let memory_session =
        match memory::begin_for_group_observed(&memory_config, group_id, gauges.clone()) {
            Ok(started) => started,
            Err(error) => return unavailable_or_error(process, error, ctx),
        };

    if ctx.control().is_cancel_requested() {
        // The memory sampler must not outlive this phase
        let _ = memory_session.finish();

        return Err(Stop::Cancelled);
    }

    let cpu_session = match session::begin_for_group_observed(
        cpu_baseline,
        config.sample_interval,
        group_id,
        gauges.clone(),
    ) {
        Ok(started) => started,
        Err(error) => {
            // The memory sampler must not outlive this phase
            let _ = memory_session.finish();

            return unavailable_or_error(process, error, ctx);
        }
    };

    if ctx.control().is_cancel_requested() {
        let _ = cpu_session.finish();
        let _ = memory_session.finish();

        return Err(Stop::Cancelled);
    }

    ctx.baseline_measured(memory_session.baseline_bytes(), cpu_baseline);
    ctx.stage_completed(Stage::Baseline, "success", None);

    ctx.stage_started(Stage::Workload);

    let outcome = match &gauges {
        Some(gauges) => {
            let mut emit_sample = |elapsed: Duration| {
                ctx.sample(build_sample(gauges, plan.limit, elapsed));
            };

            executor::execute_cancellable(
                plan,
                || process.is_running(),
                Some(executor::Observer {
                    counters: &gauges.http,
                    interval: SAMPLE_INTERVAL,
                    on_tick: &mut emit_sample,
                }),
                ctx.control(),
                || process_group::kill_now(group_id),
            )
        }
        None => executor::execute_cancellable(
            plan,
            || process.is_running(),
            None,
            ctx.control(),
            || process_group::kill_now(group_id),
        ),
    };

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

    ctx.stage_completed(Stage::Workload, &latency.status, Some(latency.duration_ms));

    Ok(PhaseOutput {
        memory: Some(memory_finished.memory),
        cpu: cpu_finished.cpu,
        http_latency: latency,
    })
}

// A vanished target is an expected condition, a failure on a live target is an error
fn unavailable_or_error(
    process: &mut TargetProcess,
    error: String,
    ctx: &RunContext,
) -> Result<PhaseOutput, Stop> {
    if process.is_running() {
        return Err(Stop::Failed(error));
    }

    ctx.stage_completed(Stage::Baseline, skip::TARGET_EXITED, None);
    ctx.stage_skipped(Stage::Workload, skip::TARGET_EXITED);

    // Target disappeared before the workload could start
    let latency = http_latency::summarize(&[], Duration::ZERO, true);

    Ok(PhaseOutput {
        memory: None,
        cpu: None,
        http_latency: latency,
    })
}
