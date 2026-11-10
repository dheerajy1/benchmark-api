use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use super::{
    request::{StartupRequest, WorkloadRequest},
    workspace::{config::WorkspaceRequest, result::WorkspaceResult},
};

// How the project is built. The caller always gives the command, nothing is detected.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BuildRequest {
    // One command line, split into words the way a shell would without running a shell
    pub command: String,
    // Relative to the project path. Left out or empty, the runtime default is measured.
    #[serde(default)]
    pub output_dir: Option<String>,
}

// Optional command that runs in the project path when the run is over, whatever the outcome,
// for example to remove the images and containers a run left behind
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CleanupRequest {
    // One command line, split into words the way a shell would without running a shell.
    // Empty means no cleanup.
    pub command: String,
    // Left out, the default applies
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BenchmarkRequest {
    pub path: PathBuf,
    pub build: BuildRequest,
    #[serde(default)]
    pub cleanup: Option<CleanupRequest>,
    #[serde(default)]
    pub startup: Option<StartupRequest>,
    #[serde(default)]
    pub workload: Option<WorkloadRequest>,
    // Extra processes measured after the main project, empty for most projects
    #[serde(default)]
    pub workspaces: Vec<WorkspaceRequest>,
}

#[derive(Clone, Serialize)]
pub struct BenchmarkResult {
    pub run_id: Uuid,
    pub target: Target,
    pub discovery: Discovery,
    pub metrics: Metrics,
}

impl BenchmarkResult {
    // A run that measured nothing yet, the start of a result that is filled step by step
    pub fn empty(run_id: Uuid, path: String) -> Self {
        Self {
            run_id,
            target: Target { path },
            discovery: Discovery {
                runtime: None,
                build: None,
            },
            metrics: Metrics {
                artifact_size: None,
                build_time: None,
                startup_time: None,
                memory: None,
                cpu: None,
                http_latency: None,
                workspaces: Vec::new(),
            },
        }
    }
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
    // A step that did not finish before the run stopped is null, never a zero
    pub artifact_size: Option<ArtifactSize>,
    pub build_time: Option<BuildTime>,
    pub startup_time: Option<StartupTime>,
    pub memory: Option<Memory>,
    pub cpu: Option<Cpu>,
    pub http_latency: Option<HttpLatency>,
    // One entry per workspace and mode. Left out of the JSON when the run had no workspace, so
    // the response of a project without workspaces is exactly what it was before.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<WorkspaceResult>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Memory {
    pub baseline: MemoryReading,
    pub peak: MemoryReading,
    pub delta: MemoryReading,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
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
