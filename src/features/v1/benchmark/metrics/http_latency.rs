use std::time::Duration;

use crate::features::v1::benchmark::{
    model::HttpLatency, workload::observation::RequestObservation,
};

pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_TARGET_EXITED: &str = "target_exited";

// Builds the latency result from individual request observations
pub fn summarize(
    observations: &[RequestObservation],
    elapsed: Duration,
    target_exited: bool,
) -> HttpLatency {
    let total_requests = observations.len() as u64;
    let successful_requests = observations
        .iter()
        .filter(|observation| observation.success)
        .count() as u64;

    let status = if target_exited {
        STATUS_TARGET_EXITED
    } else {
        STATUS_COMPLETED
    };

    let mut durations: Vec<Duration> = observations
        .iter()
        .map(|observation| observation.duration)
        .collect();

    durations.sort_unstable();

    HttpLatency {
        status: status.to_string(),
        total_requests,
        successful_requests,
        failed_requests: total_requests - successful_requests,
        duration_ms: elapsed.as_millis() as u64,
        requests_per_second: requests_per_second(total_requests, elapsed),
        min_ms: durations.first().map(|value| to_ms(*value)),
        mean_ms: mean(&durations).map(to_ms),
        p50_ms: percentile(&durations, 50).map(to_ms),
        p95_ms: percentile(&durations, 95).map(to_ms),
        p99_ms: percentile(&durations, 99).map(to_ms),
        max_ms: durations.last().map(|value| to_ms(*value)),
    }
}

// Nearest-rank percentile over durations sorted ascending
pub fn percentile(sorted: &[Duration], percent: u64) -> Option<Duration> {
    if sorted.is_empty() {
        return None;
    }

    let count = sorted.len() as u64;
    let rank = (percent * count).div_ceil(100).clamp(1, count);

    Some(sorted[(rank - 1) as usize])
}

fn mean(durations: &[Duration]) -> Option<Duration> {
    if durations.is_empty() {
        return None;
    }

    let total: Duration = durations.iter().sum();

    Some(total / durations.len() as u32)
}

fn requests_per_second(total_requests: u64, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();

    if seconds <= 0.0 {
        return 0.0;
    }

    round_two_decimals(total_requests as f64 / seconds)
}

fn to_ms(duration: Duration) -> f64 {
    round_two_decimals(duration.as_nanos() as f64 / 1_000_000.0)
}

pub fn round_two_decimals(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
