use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::features::v1::benchmark::{
    live::{LiveGauges, gauges::CpuGauge},
    model::Cpu,
};

use super::{
    calc::{MIN_PEAK_INTERVAL, build_cpu, cpu_percent, finalize_peak},
    proc::{ProcessTicks, read_group_ticks},
    tracker::CpuTracker,
};

pub struct CpuOutcome {
    // None when no reading was possible after the start snapshot
    pub cpu: Option<Cpu>,
    // Readings taken after the start snapshot
    pub samples: u64,
    // True when the target group could no longer be read
    pub target_gone: bool,
}

struct SamplerState {
    started: Instant,
    last_instant: Instant,
    peak: Option<f64>,
    samples: u64,
    target_gone: bool,
}

struct SamplerResult {
    consumed_ticks: u64,
    // Start snapshot until the last successful reading
    window: Duration,
    peak: Option<f64>,
    samples: u64,
    target_gone: bool,
}

// Continuous CPU sampling that runs until finish, never touches the target process
pub struct CpuSession {
    baseline_percent: f64,
    stop: Sender<()>,
    handle: JoinHandle<SamplerResult>,
}

// Start snapshot and background sampling for the process group
pub fn begin_for_group(
    baseline_percent: f64,
    sample_interval: Duration,
    group_id: u32,
) -> Result<CpuSession, String> {
    begin(baseline_percent, sample_interval, move || {
        read_group_ticks(group_id)
    })
}

// Same, and every sample is also published to the live gauges
pub fn begin_for_group_observed(
    baseline_percent: f64,
    sample_interval: Duration,
    group_id: u32,
    gauges: Option<Arc<LiveGauges>>,
) -> Result<CpuSession, String> {
    begin_observed(
        baseline_percent,
        sample_interval,
        move || read_group_ticks(group_id),
        gauges,
    )
}

// Start snapshot and background sampling through the reader
pub fn begin<F>(
    baseline_percent: f64,
    sample_interval: Duration,
    reader: F,
) -> Result<CpuSession, String>
where
    F: FnMut() -> Result<Vec<ProcessTicks>, String> + Send + 'static,
{
    begin_observed(baseline_percent, sample_interval, reader, None)
}

pub fn begin_observed<F>(
    baseline_percent: f64,
    sample_interval: Duration,
    mut reader: F,
    gauges: Option<Arc<LiveGauges>>,
) -> Result<CpuSession, String>
where
    F: FnMut() -> Result<Vec<ProcessTicks>, String> + Send + 'static,
{
    // Read on the caller thread so the window begins right before the workload
    let reference = reader()?;
    let started = Instant::now();
    let tracker = CpuTracker::from_reference(&reference);

    let (stop, signal) = mpsc::channel::<()>();

    let handle = thread::Builder::new()
        .name("cpu-sampler".to_string())
        .spawn(move || {
            sample_until_stopped(tracker, started, sample_interval, signal, reader, gauges)
        })
        .map_err(|error| format!("Failed to start CPU sampler: {error}"))?;

    Ok(CpuSession {
        baseline_percent,
        stop,
        handle,
    })
}

impl CpuSession {
    // Stops sampling and finalizes average and peak
    pub fn finish(self) -> Result<CpuOutcome, String> {
        // The sampler may already have exited, so a failed send is fine
        let _ = self.stop.send(());

        let result = self
            .handle
            .join()
            .map_err(|_| "CPU sampler thread panicked".to_string())?;

        Ok(build_outcome(self.baseline_percent, result))
    }
}

fn build_outcome(baseline_percent: f64, result: SamplerResult) -> CpuOutcome {
    // Without a reading after the start snapshot there is no window to measure
    let cpu = if result.window.is_zero() {
        None
    } else {
        let average = cpu_percent(result.consumed_ticks, result.window);
        let peak = finalize_peak(result.peak, average);

        Some(build_cpu(baseline_percent, average, peak))
    };

    CpuOutcome {
        cpu,
        samples: result.samples,
        target_gone: result.target_gone,
    }
}

fn sample_until_stopped<F>(
    mut tracker: CpuTracker,
    started: Instant,
    interval: Duration,
    signal: Receiver<()>,
    mut reader: F,
    gauges: Option<Arc<LiveGauges>>,
) -> SamplerResult
where
    F: FnMut() -> Result<Vec<ProcessTicks>, String>,
{
    let mut state = SamplerState {
        started,
        last_instant: started,
        peak: None,
        samples: 0,
        target_gone: false,
    };

    loop {
        match signal.recv_timeout(interval) {
            Err(RecvTimeoutError::Timeout) => {
                if !take_sample(&mut tracker, &mut state, &mut reader, &gauges) {
                    break;
                }
            }
            Ok(()) => {
                // One last reading so the end of the workload is covered
                take_sample(&mut tracker, &mut state, &mut reader, &gauges);

                break;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    SamplerResult {
        consumed_ticks: tracker.consumed_ticks(),
        window: state.last_instant.duration_since(started),
        peak: state.peak,
        samples: state.samples,
        target_gone: state.target_gone,
    }
}

// Returns false when the target group could no longer be read
fn take_sample<F>(
    tracker: &mut CpuTracker,
    state: &mut SamplerState,
    reader: &mut F,
    gauges: &Option<Arc<LiveGauges>>,
) -> bool
where
    F: FnMut() -> Result<Vec<ProcessTicks>, String>,
{
    let Ok(reading) = reader() else {
        state.target_gone = true;

        return false;
    };

    let now = Instant::now();
    let interval = now.duration_since(state.last_instant);
    let ticks = tracker.observe(&reading);

    state.samples += 1;

    let interval_percent = (interval >= MIN_PEAK_INTERVAL).then(|| cpu_percent(ticks, interval));

    if let Some(percent) = interval_percent {
        state.peak = Some(state.peak.map_or(percent, |peak| peak.max(percent)));
    }

    state.last_instant = now;

    if let Some(gauges) = gauges {
        let window = now.duration_since(state.started);
        let average_percent =
            (!window.is_zero()).then(|| cpu_percent(tracker.consumed_ticks(), window));

        gauges.set_cpu(CpuGauge {
            interval_percent,
            average_percent,
            peak_percent: state.peak,
        });
    }

    true
}
