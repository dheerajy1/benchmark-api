use std::collections::BTreeMap;

use serde::Serialize;
use uuid::Uuid;

use crate::features::v1::benchmark::model::{BenchmarkResult, Target};

// One row of the history list, a summary and never the full benchmark
#[derive(Serialize)]
pub struct BenchmarkSummary {
    pub run_id: Uuid,
    // done, failed or cancelled
    pub status: &'static str,
    // The saved target name, otherwise the last folder of the target path
    pub display_name: String,
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
    pub build_output_bytes: Option<u64>,
    pub workspaces: Vec<WorkspaceSummary>,
    pub build_time_ms: Option<u64>,
    pub startup_time_ms: Option<u64>,
    pub memory_peak_bytes: Option<u64>,
    pub cpu_average_percent: Option<f64>,
    pub requests_per_second: Option<f64>,
    pub total_requests: Option<u64>,
    pub p95_ms: Option<f64>,
}

// One workspace and mode of the run, matched by name and mode like Compare does
#[derive(Serialize)]
pub struct WorkspaceSummary {
    pub name: String,
    pub mode: String,
    pub status: String,
    pub image_bytes: Option<u64>,
    pub disk_usage_bytes: Option<u64>,
    pub build_duration_ms: Option<u64>,
}

// The numbers of the filter pills, each follows every filter except its own facet
#[derive(Serialize)]
pub struct Counts {
    // Runs matching every filter
    pub total: u64,
    // Per runtime code, a run without a runtime is under "unknown", runtimes without a run are absent
    pub by_runtime: BTreeMap<String, u64>,
    pub by_status: StatusCounts,
}

#[derive(Serialize)]
pub struct StatusCounts {
    pub done: u64,
    pub failed: u64,
    pub cancelled: u64,
}

// One page of history plus what the client needs to ask for the next one
#[derive(Serialize)]
pub struct BenchmarkPage {
    pub items: Vec<BenchmarkSummary>,
    pub counts: Counts,
    pub pagination: Pagination,
}

#[derive(Serialize)]
pub struct Pagination {
    pub limit: u32,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

// One stored run: the benchmark as it was measured, how it ended and how long each stage took.
// A part that did not finish is null, never a zero.
#[derive(Serialize)]
pub struct BenchmarkDetail {
    #[serde(flatten)]
    pub result: BenchmarkResult,
    // done, failed or cancelled
    pub status: &'static str,
    // The stage a failed or cancelled run stopped in, null for a done run
    pub stopped_stage: Option<String>,
    // Why a failed run failed, null for the others
    pub failure_message: Option<String>,
    // The saved target name, otherwise the last folder of the target path
    pub display_name: String,
    // In run order, empty when the run has no stage rows
    pub stages: Vec<StageEntry>,
}

#[derive(Serialize)]
pub struct StageEntry {
    pub stage: String,
    pub status: String,
    pub duration_ms: Option<u64>,
}
