use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use super::request::{StartupRequest, WorkloadRequest};

#[derive(Debug, Clone, Deserialize)]
pub struct BenchmarkRequest {
    pub path: PathBuf,
    #[serde(default)]
    pub startup: Option<StartupRequest>,
    #[serde(default)]
    pub workload: Option<WorkloadRequest>,
}

#[derive(Clone, Serialize)]
pub struct BenchmarkResult {
    pub run_id: Uuid,
    pub target: Target,
    pub discovery: Discovery,
    pub metrics: Metrics,
}

#[derive(Clone, Serialize)]
pub struct Target {
    pub path: String,
}

#[derive(Clone, Serialize)]
pub struct Discovery {
    pub runtime: Option<String>,
    pub build: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct Metrics {
    pub artifact_size: ArtifactSize,
    pub build_time: BuildTime,
    pub startup_time: StartupTime,
    pub memory: Option<Memory>,
    pub cpu: Option<Cpu>,
    pub http_latency: Option<HttpLatency>,
}

#[derive(Clone, Serialize)]
pub struct ArtifactSize {
    pub source: ArtifactSizeCategory,
    pub build: ArtifactSizeCategory,
    pub excluded: ArtifactSizeCategory,
}

#[derive(Clone, Serialize)]
pub struct ArtifactSizeCategory {
    pub bytes: u64,
    pub size: String,
    pub file_count: u64,
}

#[derive(Clone, Serialize)]
pub struct BuildTime {
    pub status: String,
    pub duration_ms: Option<u64>,
    pub command: Option<BuildCommand>,
    pub working_directory: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Serialize)]
pub struct StartupTime {
    pub status: String,
    pub duration_ms: Option<u64>,
    pub command: Option<BuildCommand>,
    pub working_directory: Option<String>,
    pub readiness: Option<StartupReadiness>,
    pub started_at: Option<String>,
    pub ready_at: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Serialize)]
pub struct StartupReadiness {
    #[serde(rename = "type")]
    pub readiness_type: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub method: String,
    pub expected_status: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct Memory {
    pub baseline: MemoryReading,
    pub peak: MemoryReading,
    pub delta: MemoryReading,
}

#[derive(Clone, Serialize)]
pub struct MemoryReading {
    pub rss_bytes: u64,
    pub rss_size: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cpu {
    // Process-group CPU right before the workload, 100.0 is one fully used core
    pub baseline_percent: f64,
    // CPU time during the workload over workload wall-clock time
    pub average_percent: f64,
    // Highest sampled CPU during the workload
    pub peak_percent: f64,
}

#[derive(Clone, Serialize)]
pub struct HttpLatency {
    // "completed" or "target_exited"
    pub status: String,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub duration_ms: u64,
    pub requests_per_second: f64,
    pub min_ms: Option<f64>,
    pub mean_ms: Option<f64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub max_ms: Option<f64>,
}
