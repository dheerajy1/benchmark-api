use chrono::{DateTime, Utc};
use uuid::Uuid;

// Database shaped rows: canonical numbers, typed timestamps, no formatted strings

pub struct BenchmarkRows {
    pub run: RunRow,
    pub artifact_sizes: Vec<ArtifactSizeRow>,
    pub build: BuildRow,
    pub startup: StartupRow,
    pub memory: Option<MemoryRow>,
    pub cpu: Option<CpuRow>,
    pub http_latency: Option<HttpLatencyRow>,
    pub workspaces: Vec<WorkspaceRow>,
}

pub struct RunRow {
    pub id: Uuid,
    pub target_path: String,
    pub runtime_code: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub struct ArtifactSizeRow {
    pub artifact_type_code: &'static str,
    pub bytes: i64,
    pub file_count: i64,
}

pub struct BuildRow {
    pub status_code: String,
    pub duration_ms: Option<i64>,
    pub program: Option<String>,
    pub args: Option<Vec<String>>,
    pub working_directory: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
}

pub struct StartupRow {
    pub status_code: String,
    pub duration_ms: Option<i64>,
    pub program: Option<String>,
    pub args: Option<Vec<String>>,
    pub working_directory: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ready_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub readiness_type: Option<String>,
    pub readiness_host: Option<String>,
    pub readiness_port: Option<i32>,
    pub readiness_path: Option<String>,
    pub readiness_method: Option<String>,
    pub readiness_expected_status: Option<i32>,
}

pub struct MemoryRow {
    pub baseline_rss_bytes: i64,
    pub peak_rss_bytes: i64,
    pub delta_rss_bytes: i64,
}

pub struct CpuRow {
    pub baseline_percent: f64,
    pub average_percent: f64,
    pub peak_percent: f64,
}

pub struct HttpLatencyRow {
    pub status_code: String,
    pub total_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub duration_ms: i64,
    pub requests_per_second: f64,
    pub min_ms: Option<f64>,
    pub mean_ms: Option<f64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub max_ms: Option<f64>,
}

// One mode of one workspace. The memory and CPU rows are the same shape the main project uses.
pub struct WorkspaceRow {
    pub name: String,
    pub mode: String,
    pub status_code: String,
    pub message: Option<String>,
    pub env_variable_names: Vec<String>,
    pub build_status_code: Option<String>,
    pub build_duration_ms: Option<i64>,
    pub build_exit_code: Option<i32>,
    pub image_bytes: Option<i64>,
    pub image_disk_usage_bytes: Option<i64>,
    pub startup_ms: Option<i64>,
    pub memory: Option<MemoryRow>,
    pub cpu: Option<CpuRow>,
    pub load_status_code: Option<String>,
    pub load_exit_code: Option<i32>,
    pub load_duration_ms: Option<i64>,
    pub idle_seconds: Option<i64>,
}
