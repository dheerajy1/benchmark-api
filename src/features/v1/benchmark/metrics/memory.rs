use std::{
    fs, thread,
    time::{Duration, Instant},
};

use crate::features::v1::benchmark::{
    metrics::artifact_size::format_size,
    model::{Memory, MemoryReading},
    target_process::TargetProcess,
};

pub const DEFAULT_STABILIZATION: Duration = Duration::from_millis(500);
pub const DEFAULT_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
pub const DEFAULT_SAMPLE_WINDOW: Duration = Duration::from_secs(5);

pub struct MemoryConfig {
    // Wait between readiness and the baseline reading
    pub stabilization: Duration,
    // Time between peak samples
    pub sample_interval: Duration,
    // Total time spent sampling for the peak after the baseline
    pub sample_window: Duration,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            stabilization: DEFAULT_STABILIZATION,
            sample_interval: DEFAULT_SAMPLE_INTERVAL,
            sample_window: DEFAULT_SAMPLE_WINDOW,
        }
    }
}

// Measures a running target, never shuts the process down
pub fn measure(process: &mut TargetProcess) -> Result<Memory, String> {
    measure_with_config(process, &MemoryConfig::default())
}

pub fn measure_with_config(
    process: &mut TargetProcess,
    config: &MemoryConfig,
) -> Result<Memory, String> {
    let group_id = process.process_group_id();

    measure_with_sampler(config, || {
        if !process.is_running() {
            return Err("Target process exited during memory measurement".to_string());
        }

        read_group_rss(group_id)
    })
}

// Baseline after stabilization, then peak over the sample window
pub fn measure_with_sampler<F>(config: &MemoryConfig, mut sampler: F) -> Result<Memory, String>
where
    F: FnMut() -> Result<u64, String>,
{
    thread::sleep(config.stabilization);

    let baseline = sampler()?;
    let mut peak = baseline;

    let started = Instant::now();

    while started.elapsed() < config.sample_window {
        thread::sleep(config.sample_interval);

        let sample = sampler()?;

        if sample > peak {
            peak = sample;
        }
    }

    Ok(build_memory(baseline, peak))
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
