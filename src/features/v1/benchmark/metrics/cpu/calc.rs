use std::time::Duration;

use crate::features::v1::benchmark::{metrics::http_latency::round_two_decimals, model::Cpu};

use super::proc::USER_HZ;

// Intervals shorter than this are too coarse for a USER_HZ based peak
pub const MIN_PEAK_INTERVAL: Duration = Duration::from_millis(50);

// CPU time over wall-clock time, 100.0 is one fully used core
pub fn cpu_percent(ticks: u64, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();

    if seconds <= 0.0 {
        return 0.0;
    }

    let cpu_seconds = ticks as f64 / USER_HZ as f64;

    cpu_seconds / seconds * 100.0
}

// Peak is never below the average, and equals it when no valid sample exists
pub fn finalize_peak(sampled_peak: Option<f64>, average: f64) -> f64 {
    sampled_peak.unwrap_or(0.0).max(average)
}

// Rounds every value to two decimals
pub fn build_cpu(baseline_percent: f64, average_percent: f64, peak_percent: f64) -> Cpu {
    Cpu {
        baseline_percent: round_two_decimals(baseline_percent),
        average_percent: round_two_decimals(average_percent),
        peak_percent: round_two_decimals(peak_percent),
    }
}
