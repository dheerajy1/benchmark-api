use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::outcome::StoredStatus;

// Position of the last row of a page, the next page holds rows strictly older than it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPosition {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

// Filters and pagination of the history list, applied by the database
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SummaryQuery {
    // Maximum number of rows to return
    pub limit: i64,
    // Only rows strictly older than this position, in (created_at, id) order
    pub after: Option<CursorPosition>,
    // Lookup code of the runtime, for example bun
    pub runtime_code: Option<String>,
    // Exact match on the stored target path
    pub target_path: Option<String>,
    // Only runs that ended with this status
    pub status: Option<StoredStatus>,
    // Case-insensitive text found in the target path or anywhere in the run id
    pub search: Option<String>,
}

// What the filter pills show, every figure follows the filters of the query except the one
// of the facet it describes, the cursor and the limit never count
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryCounts {
    // Runs matching every filter
    pub total: i64,
    // Runs per runtime code, a run without a runtime is under "unknown"
    pub by_runtime: Vec<(String, i64)>,
    // Runs per status, all three statuses are present
    pub by_status: Vec<(StoredStatus, i64)>,
}

// One workspace mode of a run in the history list, every number is null when it was not made
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSummaryRow {
    pub name: String,
    pub mode: String,
    pub status: String,
    pub image_bytes: Option<i64>,
    pub disk_usage_bytes: Option<i64>,
    pub build_duration_ms: Option<i64>,
}

// One history row, canonical numbers straight from the normalized tables
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryRow {
    pub id: Uuid,
    pub target_path: String,
    // The saved target name from the run configuration, None for a custom path
    pub target_name: Option<String>,
    pub runtime_code: Option<String>,
    pub status: StoredStatus,
    pub created_at: DateTime<Utc>,
    pub build_output_bytes: Option<i64>,
    pub workspaces: Vec<WorkspaceSummaryRow>,
    pub build_time_ms: Option<i64>,
    pub startup_time_ms: Option<i64>,
    pub memory_peak_bytes: Option<i64>,
    pub cpu_average_percent: Option<f64>,
    pub requests_per_second: Option<f64>,
    pub p95_ms: Option<f64>,
}

// One overview trend row, comparison rules already applied by the database:
// a build or startup duration is only set when that step succeeded and the HTTP numbers
// are only set for a completed workload with at least one request, otherwise None
#[derive(Debug, Clone, PartialEq)]
pub struct TrendRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub build_time_ms: Option<i64>,
    pub startup_time_ms: Option<i64>,
    pub memory_peak_bytes: Option<i64>,
    pub cpu_average_percent: Option<f64>,
    pub requests_per_second: Option<f64>,
    pub p95_ms: Option<f64>,
}

// Which runs the overview trend reads. The comparison rules always apply, the filters only
// choose the runs: a project, a status or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrendQuery {
    pub limit: i64,
    // Exact match on the stored target path
    pub target_path: Option<String>,
    // Only runs that ended with this status
    pub status: Option<StoredStatus>,
}

impl TrendQuery {
    // The newest runs of every project and status
    pub fn all(limit: i64) -> Self {
        Self {
            limit,
            target_path: None,
            status: None,
        }
    }
}

// One project of the overview switcher: a stored target path with what its runs say about it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub path: String,
    // The saved target name of the newest run, None for a custom path
    pub target_name: Option<String>,
    // The runtime of the newest run that has one
    pub runtime_code: Option<String>,
    // target or custom, from the configuration of the newest run
    pub source: Option<String>,
    pub total_runs: i64,
    pub done_runs: i64,
    pub last_run_at: DateTime<Utc>,
}

// How the runs of one project ended, deleted runs never count
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectTotals {
    pub total_runs: i64,
    pub done_runs: i64,
    pub failed_runs: i64,
    pub cancelled_runs: i64,
}

// One stage of a stored run with its duration, in run order
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageDuration {
    pub stage: String,
    pub position: i32,
    // None when the stage has no duration, which is never read as a zero
    pub duration_ms: Option<i64>,
}

// The stages of one done run, used for the stage averages. A run without stage rows has an
// empty list and still counts as a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRun {
    pub run_id: Uuid,
    pub stages: Vec<StageDuration>,
}
