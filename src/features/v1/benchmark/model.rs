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
