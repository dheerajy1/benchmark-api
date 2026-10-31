use std::{
    fs,
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::features::v1::benchmark::{
    live::LiveGauges,
    metrics::artifact_size::format_size,
    model::{Memory, MemoryReading},
};

pub const DEFAULT_STABILIZATION: Duration = Duration::from_millis(500);
pub const DEFAULT_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    // Wait between readiness and the baseline reading
    pub stabilization: Duration,
    // Time between RSS samples while the workload runs
    pub sample_interval: Duration,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            stabilization: DEFAULT_STABILIZATION,
            sample_interval: DEFAULT_SAMPLE_INTERVAL,
        }
    }
}

pub struct MemoryOutcome {
    pub memory: Memory,
    // Samples taken after the baseline
    pub samples: u64,
    // True when RSS could no longer be read because the target group disappeared
    pub target_gone: bool,
}

struct SamplerResult {
    peak: u64,
    samples: u64,
    target_gone: bool,
}

// Continuous RSS sampling that runs until finish, never touches the target process
pub struct MemorySession {
    baseline: u64,
    stop: Sender<()>,
    handle: JoinHandle<SamplerResult>,
}

// Baseline after stabilization, then background sampling for the process group
pub fn begin_for_group(config: &MemoryConfig, group_id: u32) -> Result<MemorySession, String> {
    begin(config, move || read_group_rss(group_id))
}

// Same, and every reading is also published to the live gauges
pub fn begin_for_group_observed(
    config: &MemoryConfig,
    group_id: u32,
    gauges: Option<Arc<LiveGauges>>,
) -> Result<MemorySession, String> {
    begin_observed(config, move || read_group_rss(group_id), gauges)
}

// Baseline after stabilization, then background sampling through the reader
pub fn begin<F>(config: &MemoryConfig, reader: F) -> Result<MemorySession, String>
where
    F: FnMut() -> Result<u64, String> + Send + 'static,
{
    begin_observed(config, reader, None)
}

pub fn begin_observed<F>(
    config: &MemoryConfig,
    mut reader: F,
    gauges: Option<Arc<LiveGauges>>,
) -> Result<MemorySession, String>
where
    F: FnMut() -> Result<u64, String> + Send + 'static,
{
    thread::sleep(config.stabilization);

    let baseline = reader()?;

    if let Some(gauges) = &gauges {
        gauges.set_memory(baseline, baseline);
    }

    let (stop, signal) = mpsc::channel::<()>();
    let interval = config.sample_interval;

    let handle = thread::Builder::new()
        .name("memory-sampler".to_string())
        .spawn(move || sample_until_stopped(baseline, interval, signal, reader, gauges))
        .map_err(|error| format!("Failed to start memory sampler: {error}"))?;

    Ok(MemorySession {
        baseline,
        stop,
        handle,
    })
}

impl MemorySession {
    // The RSS read right after stabilization, available while sampling is still running
    pub fn baseline_bytes(&self) -> u64 {
        self.baseline
    }

    // Stops sampling and finalizes peak and delta
    pub fn finish(self) -> Result<MemoryOutcome, String> {
        // The sampler may already have exited, so a failed send is fine
        let _ = self.stop.send(());

        let result = self
            .handle
            .join()
            .map_err(|_| "Memory sampler thread panicked".to_string())?;

        Ok(MemoryOutcome {
            memory: build_memory(self.baseline, result.peak),
            samples: result.samples,
            target_gone: result.target_gone,
        })
    }
}

fn sample_until_stopped<F>(
    baseline: u64,
    interval: Duration,
    signal: Receiver<()>,
    mut reader: F,
    gauges: Option<Arc<LiveGauges>>,
) -> SamplerResult
where
    F: FnMut() -> Result<u64, String>,
{
    let mut result = SamplerResult {
        peak: baseline,
        samples: 0,
        target_gone: false,
    };

    loop {
        match signal.recv_timeout(interval) {
            Err(RecvTimeoutError::Timeout) => match reader() {
                Ok(sample) => {
                    record(&mut result, sample);

                    if let Some(gauges) = &gauges {
                        gauges.set_memory(sample, result.peak);
                    }
                }
                Err(_) => {
                    result.target_gone = true;
                    break;
                }
            },
            Ok(()) => {
                // One last reading so the end of the workload is covered
                match reader() {
                    Ok(sample) => record(&mut result, sample),
                    Err(_) => result.target_gone = true,
                }

                break;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    result
}

fn record(result: &mut SamplerResult, sample: u64) {
    result.samples += 1;

    if sample > result.peak {
        result.peak = sample;
    }
}

pub fn build_memory(baseline_bytes: u64, peak_bytes: u64) -> Memory {
    Memory {
        baseline: reading(baseline_bytes),
        peak: reading(peak_bytes),
        delta: reading(peak_bytes.saturating_sub(baseline_bytes)),
    }
}

fn reading(rss_bytes: u64) -> MemoryReading {
    MemoryReading {
        rss_bytes,
        rss_size: format_size(rss_bytes),
    }
}

// Sums RSS of every process whose process group id matches
pub fn read_group_rss(group_id: u32) -> Result<u64, String> {
    let entries =
        fs::read_dir("/proc").map_err(|error| format!("Failed to read /proc: {error}"))?;

    let mut matched = 0u32;
    let mut total = 0u64;

    for entry in entries.flatten() {
        let name = entry.file_name();

        let Some(pid) = name.to_str().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };

        // A process may exit between listing and reading, skip it
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };

        if parse_group_id(&stat) != Some(group_id) {
            continue;
        }

        matched += 1;

        if let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status"))
            && let Some(bytes) = parse_vm_rss_bytes(&status)
        {
            total += bytes;
        }
    }

    if matched == 0 {
        return Err("No live processes found in the target process group".to_string());
    }

    Ok(total)
}

// Sums RSS of a process and everything below it, used for a container's process tree
pub fn read_tree_rss(root: u32) -> Result<u64, String> {
    let members = super::process_tree::tree_members(root)?;

    let mut total = 0u64;

    for pid in members {
        // A process may exit between listing and reading, skip it
        if let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status"))
            && let Some(bytes) = parse_vm_rss_bytes(&status)
        {
            total += bytes;
        }
    }

    Ok(total)
}

// Field 5 of /proc/<pid>/stat, parsed after the last ')' because comm may contain spaces
pub fn parse_group_id(stat: &str) -> Option<u32> {
    let end = stat.rfind(')')?;

    stat[end + 1..]
        .split_whitespace()
        .nth(2)?
        .parse::<u32>()
        .ok()
}

// VmRSS in /proc/<pid>/status is reported in kB
pub fn parse_vm_rss_bytes(status: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;

    let kilobytes = line
        .trim_start_matches("VmRSS:")
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;

    Some(kilobytes * 1024)
}
