use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct BenchmarkRequest {
    pub path: PathBuf,
}

#[derive(Serialize)]
pub struct BenchmarkResult {
    pub run_id: Uuid,
    pub target: Target,
    pub discovery: Discovery,
    pub metrics: Metrics,
}

#[derive(Serialize)]
pub struct Target {
    pub path: String,
}

#[derive(Serialize)]
pub struct Discovery {
    pub runtime: Option<String>,
    pub build: Option<String>,
}

#[derive(Serialize)]
pub struct Metrics {
    pub artifact_size: ArtifactSize,
    pub build_time: BuildTime,
    pub startup_time: StartupTime,
}

#[derive(Serialize)]
pub struct ArtifactSize {
    pub source: ArtifactSizeCategory,
    pub build: ArtifactSizeCategory,
    pub excluded: ArtifactSizeCategory,
}

#[derive(Serialize)]
pub struct ArtifactSizeCategory {
    pub bytes: u64,
    pub size: String,
    pub file_count: u64,
}

#[derive(Serialize)]
pub struct BuildTime {
    pub status: String,
    pub duration_ms: Option<u64>,
    pub command: Option<BuildCommand>,
    pub working_directory: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Serialize)]
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

#[derive(Serialize)]
pub struct StartupReadiness {
    #[serde(rename = "type")]
    pub readiness_type: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub method: String,
    pub expected_status: u16,
}

#[derive(Serialize)]
pub struct BuildCommand {
    pub program: String,
    pub args: Vec<String>,
}
