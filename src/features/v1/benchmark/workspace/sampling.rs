use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        mpsc::{self, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::features::v1::benchmark::{
    live::{LiveGauges, RunContext, RunControl, SAMPLE_INTERVAL, gauges::build_workspace_sample},
    metrics::{
        artifact_size::format_size,
        cpu::{
            baseline,
            proc::{ProcessTicks, read_group_ticks, read_tree_ticks},
            session,
        },
        memory::{self, MemoryConfig, read_group_rss, read_tree_rss},
    },
    model::{BuildCommand, Cpu, Memory},
    stop::{Stop, sleep_unless_cancelled},
};

use super::{
    exec::{self, Finished, Spec},
    result::LoadResult,
};

// A load command that runs longer than this is stopped
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(60 * 60);

// Which processes are measured: a process group (native, started under setsid) or a process
// and its children (the main process of a container, as the host sees it)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Group(u32),
    Tree(u32),
}

impl Scope {
    fn rss(self) -> Result<u64, String> {
        match self {
            Scope::Group(id) => read_group_rss(id),
            Scope::Tree(id) => read_tree_rss(id),
        }
    }

    fn ticks(self) -> Result<Vec<ProcessTicks>, String> {
        match self {
            Scope::Group(id) => read_group_ticks(id),
            Scope::Tree(id) => read_tree_ticks(id),
        }
    }
}

// What the sampling window produced
pub struct Window {
    pub memory: Memory,
    pub cpu: Option<Cpu>,
    pub load: Option<LoadResult>,
    // Some when the window was an idle one
    pub idle_seconds: Option<u64>,
    // True when the measured processes ended during the window
    pub target_gone: bool,
}

// What runs while memory and CPU are sampled
pub struct Activity<'a> {
    // Where the live events go, and which workspace mode they describe
    pub ctx: &'a RunContext,
    pub workspace: &'a str,
    pub mode: &'a str,
    pub load: Option<&'a BuildCommand>,
    pub directory: &'a Path,
    pub env: &'a BTreeMap<String, String>,
    pub idle_seconds: u64,
}

// Stabilization, baselines, then sampling every interval while the load command runs, or for
// the idle time when there is none. The same sampler serves native and Docker, so the
// numbers are measured the same way.
pub fn measure(
    scope: Scope,
    config: &MemoryConfig,
    activity: &Activity<'_>,
    control: &RunControl,
) -> Result<Window, Stop> {
    if sleep_unless_cancelled(config.stabilization, control) {
        return Err(Stop::Cancelled);
    }

    let cpu_baseline = baseline::measure_baseline(config.sample_interval, move || scope.ticks())
        .map_err(Stop::Failed)?;

    let memory_config = MemoryConfig {
        stabilization: Duration::ZERO,
        sample_interval: config.sample_interval,
    };

    // Only a run somebody watches publishes readings, an unobserved run does no extra work
    let gauges = activity
        .ctx
        .samples_enabled()
        .then(|| Arc::new(LiveGauges::default()));

    let memory_session =
        memory::begin_observed(&memory_config, move || scope.rss(), gauges.clone())
            .map_err(Stop::Failed)?;

    let cpu_session = match session::begin_observed(
        cpu_baseline,
        config.sample_interval,
        move || scope.ticks(),
        gauges.clone(),
    ) {
        Ok(started) => started,
        Err(message) => {
            // The memory sampler must not outlive this window
            let _ = memory_session.finish();

            return Err(Stop::Failed(message));
        }
    };

    activity
        .ctx
        .workspace_phase(activity.workspace, activity.mode, "sampling", None);

    let ticker = gauges.map(|gauges| Ticker::start(activity, gauges));

    let activity_result = run_activity(activity, control);

    if let Some(ticker) = ticker {
        ticker.stop();
    }

    // Samplers are always stopped, even when the load failed or was cancelled
    let cpu_finished = cpu_session.finish();
    let memory_finished = memory_session.finish();

    let (load, idle_seconds) = activity_result?;
    let cpu_finished = cpu_finished.map_err(Stop::Failed)?;
    let memory_finished = memory_finished.map_err(Stop::Failed)?;

    Ok(Window {
        memory: memory_finished.memory,
        cpu: cpu_finished.cpu,
        load,
        idle_seconds,
        target_gone: memory_finished.target_gone || cpu_finished.target_gone,
    })
}

// Publishes the latest readings every SAMPLE_INTERVAL while the window is open. The samplers
// read every 100 ms, this only forwards their latest values.
struct Ticker {
    stop: Sender<()>,
    handle: JoinHandle<()>,
}

impl Ticker {
    fn start(activity: &Activity<'_>, gauges: Arc<LiveGauges>) -> Self {
        let ctx = activity.ctx.clone();
        let workspace = activity.workspace.to_string();
        let mode = activity.mode.to_string();
        let (stop, signal) = mpsc::channel::<()>();

        let handle = thread::spawn(move || {
            let started = Instant::now();

            while let Err(RecvTimeoutError::Timeout) = signal.recv_timeout(SAMPLE_INTERVAL) {
                ctx.workspace_sample(build_workspace_sample(
                    &gauges,
                    &workspace,
                    &mode,
                    started.elapsed(),
                ));
            }
        });

        Self { stop, handle }
    }

    fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.handle.join();
    }
}

fn run_activity(
    activity: &Activity<'_>,
    control: &RunControl,
) -> Result<(Option<LoadResult>, Option<u64>), Stop> {
    let Some(load) = activity.load else {
        if sleep_unless_cancelled(Duration::from_secs(activity.idle_seconds), control) {
            return Err(Stop::Cancelled);
        }

        return Ok((None, Some(activity.idle_seconds)));
    };

    let timed = exec::run_to_completion(
        &Spec {
            command: load,
            directory: activity.directory,
            env: activity.env,
        },
        LOAD_TIMEOUT,
        control,
    )?;

    let status = match timed.finished {
        Finished::Exited(Some(0)) => "success",
        Finished::Exited(_) => "failed",
        Finished::TimedOut => "timeout",
    };

    Ok((
        Some(LoadResult {
            status: status.to_string(),
            exit_code: timed.exit_code(),
            duration_ms: timed.duration_ms(),
        }),
        None,
    ))
}

pub fn size_of(bytes: u64, disk_usage_bytes: Option<u64>) -> super::result::ImageSize {
    super::result::ImageSize {
        bytes,
        size: format_size(bytes),
        disk_usage_bytes,
        disk_usage: disk_usage_bytes.map(format_size),
    }
}
