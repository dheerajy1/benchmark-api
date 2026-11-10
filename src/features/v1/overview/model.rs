use serde::Serialize;
use uuid::Uuid;

use crate::features::v1::benchmark::history::model::BenchmarkSummary;

// The dashboard of one project: how its runs ended, its newest two done runs and the recent
// trend of its done runs. Every part belongs to the same project.
#[derive(Serialize)]
pub struct Overview {
    // The project shown, null only when no run is stored at all
    pub project: Option<Project>,
    // Every project that has a run, newest activity first, for the switcher
    pub projects: Vec<ProjectEntry>,
    pub totals: Totals,
    // The newest done run of the project in the history summary form, null when it has none
    pub latest: Option<BenchmarkSummary>,
    // The done run before the latest, so the changes shown need no second call
    pub previous: Option<BenchmarkSummary>,
    // The newest done runs of the project, newest first
    pub trends: Vec<TrendPoint>,
}

#[derive(Serialize)]
pub struct Project {
    pub path: String,
    // The saved target name, otherwise the last folder of the path
    pub name: String,
    // The runtime of the newest run that has one, null when none has
    pub runtime: Option<String>,
    // target or custom, null for a path without runs
    pub source: Option<String>,
}

#[derive(Serialize)]
pub struct ProjectEntry {
    pub path: String,
    pub name: String,
    pub runtime: Option<String>,
    pub total_runs: u64,
    pub done_runs: u64,
    pub last_run_at: String,
}

// How the runs of the project ended, deleted runs are not counted
#[derive(Serialize)]
pub struct Totals {
    pub total_runs: u64,
    pub done_runs: u64,
    pub failed_runs: u64,
    pub cancelled_runs: u64,
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
    // The requests the workload sent, null under the same rule as requests_per_second
    pub total_requests: Option<u64>,
    pub p95_ms: Option<f64>,
}
