use serde::Serialize;
use uuid::Uuid;

use crate::features::v1::benchmark::model::Target;

use super::metric::{DecimalMetric, IntegerMetric};

// The purpose-built comparison response, never the raw database rows
#[derive(Serialize)]
pub struct ComparisonResponse {
    pub baseline: RunReference,
    pub comparison: RunReference,
    pub metrics: ComparisonMetrics,
}

// Identifies one side of the comparison
#[derive(Serialize)]
pub struct RunReference {
    pub run_id: Uuid,
    pub target: Target,
    pub runtime: Option<String>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct ComparisonMetrics {
    pub build_time: BuildTimeComparison,
    pub startup_time: StartupTimeComparison,
    pub memory: MemoryComparison,
    pub cpu: CpuComparison,
    pub http_latency: HttpLatencyComparison,
    pub artifact_size: ArtifactSizeComparison,
    // One entry per workspace and mode that either run has, left out when neither had one
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<WorkspaceComparison>,
}

// The statuses explain why a side is null, for example no_build_command
#[derive(Serialize)]
pub struct BuildTimeComparison {
    pub baseline_status: String,
    pub comparison_status: String,
    pub duration_ms: IntegerMetric,
}

#[derive(Serialize)]
pub struct StartupTimeComparison {
    pub baseline_status: String,
    pub comparison_status: String,
    pub duration_ms: IntegerMetric,
}

#[derive(Serialize)]
pub struct MemoryComparison {
    pub baseline_rss_bytes: IntegerMetric,
    pub peak_rss_bytes: IntegerMetric,
    pub delta_rss_bytes: IntegerMetric,
}

#[derive(Serialize)]
pub struct CpuComparison {
    pub baseline_percent: DecimalMetric,
    pub average_percent: DecimalMetric,
    pub peak_percent: DecimalMetric,
}

// A status is null when the run had no HTTP workload
#[derive(Serialize)]
pub struct HttpLatencyComparison {
    pub baseline_status: Option<String>,
    pub comparison_status: Option<String>,
    pub requests_per_second: DecimalMetric,
    pub p50_ms: DecimalMetric,
    pub p95_ms: DecimalMetric,
    pub p99_ms: DecimalMetric,
}

#[derive(Serialize)]
pub struct ArtifactSizeComparison {
    pub source: ArtifactCategoryComparison,
    pub build: ArtifactCategoryComparison,
    pub excluded: ArtifactCategoryComparison,
}

#[derive(Serialize)]
pub struct ArtifactCategoryComparison {
    pub bytes: IntegerMetric,
    pub file_count: IntegerMetric,
}

// One workspace in one mode (native or docker), matched by name and mode across the two runs.
// A status is null when that run did not have the workspace in that mode. Numbers only count
// for a mode that succeeded, a failed one is not a comparable result.
#[derive(Serialize)]
pub struct WorkspaceComparison {
    pub name: String,
    pub mode: String,
    pub baseline_status: Option<String>,
    pub comparison_status: Option<String>,
    pub image_bytes: IntegerMetric,
    pub build_duration_ms: IntegerMetric,
    pub startup_duration_ms: IntegerMetric,
    pub baseline_rss_bytes: IntegerMetric,
    pub peak_rss_bytes: IntegerMetric,
    pub delta_rss_bytes: IntegerMetric,
    pub cpu_baseline_percent: DecimalMetric,
    pub cpu_average_percent: DecimalMetric,
    pub cpu_peak_percent: DecimalMetric,
}
