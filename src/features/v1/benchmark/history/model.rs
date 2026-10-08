use serde::Serialize;
use uuid::Uuid;

use crate::features::v1::benchmark::model::Target;

// One row of the history list, a summary and never the full benchmark
#[derive(Serialize)]
pub struct BenchmarkSummary {
    pub run_id: Uuid,
    pub target: Target,
    pub discovery: SummaryDiscovery,
    pub created_at: String,
    pub summary: SummaryMetrics,
}

#[derive(Serialize)]
pub struct SummaryDiscovery {
    pub runtime: Option<String>,
}

// Every metric is null when the run did not produce it
#[derive(Serialize)]
pub struct SummaryMetrics {
    pub build_time_ms: Option<u64>,
    pub startup_time_ms: Option<u64>,
    pub memory_peak_bytes: Option<u64>,
    pub cpu_average_percent: Option<f64>,
    pub requests_per_second: Option<f64>,
    pub p95_ms: Option<f64>,
}

// One page of history plus what the client needs to ask for the next one
#[derive(Serialize)]
pub struct BenchmarkPage {
    pub items: Vec<BenchmarkSummary>,
    pub pagination: Pagination,
}

#[derive(Serialize)]
pub struct Pagination {
    pub limit: u32,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}
