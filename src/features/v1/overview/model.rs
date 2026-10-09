use serde::Serialize;
use uuid::Uuid;

use crate::features::v1::benchmark::history::model::BenchmarkSummary;

// The dashboard aggregate: complete history counts, the newest run and the recent trend
#[derive(Serialize)]
pub struct Overview {
    pub totals: Totals,
    // The existing history summary of the newest run, null when no run is stored
    pub latest: Option<BenchmarkSummary>,
    // The newest runs of any kind, newest first
    pub trends: Vec<TrendPoint>,
}

#[derive(Serialize)]
pub struct Totals {
    // Every persisted run, there is no run level status to count successes with
    pub total_runs: u64,
}

// One run of the trend. A metric is null when it was not measured, did not succeed or does
// not apply, and a null is never a zero.
#[derive(Serialize)]
pub struct TrendPoint {
    pub run_id: Uuid,
    pub created_at: String,
    pub build_time_ms: Option<u64>,
    pub startup_time_ms: Option<u64>,
    pub memory_peak_bytes: Option<u64>,
    pub cpu_average_percent: Option<f64>,
    pub requests_per_second: Option<f64>,
    pub p95_ms: Option<f64>,
}
