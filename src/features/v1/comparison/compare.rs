use chrono::SecondsFormat;

use crate::{
    features::v1::benchmark::model::{ArtifactSizeCategory, BenchmarkResult, HttpLatency},
    persistence::stored_run::StoredRun,
};

use super::{
    metric::{self, Direction},
    model::{
        ArtifactCategoryComparison, ArtifactSizeComparison, BuildTimeComparison, ComparisonMetrics,
        ComparisonResponse, CpuComparison, HttpLatencyComparison, MemoryComparison, RunReference,
        StartupTimeComparison,
    },
};

// Build and startup durations only count when the step succeeded, HTTP only when the
// workload completed. A failed or partial measurement is not a comparable result.
const STATUS_SUCCESS: &str = "success";
const STATUS_COMPLETED: &str = "completed";

// Pure calculation, no storage access: two historical runs in, one comparison out
pub fn compare(baseline: &StoredRun, comparison: &StoredRun) -> ComparisonResponse {
    let (base, comp) = (&baseline.result, &comparison.result);

    ComparisonResponse {
        baseline: reference(baseline),
        comparison: reference(comparison),
        metrics: ComparisonMetrics {
            build_time: BuildTimeComparison {
                baseline_status: base.metrics.build_time.status.clone(),
                comparison_status: comp.metrics.build_time.status.clone(),
                duration_ms: metric::integer(
                    build_duration(base),
                    build_duration(comp),
                    Direction::LowerIsBetter,
                ),
            },
            startup_time: StartupTimeComparison {
                baseline_status: base.metrics.startup_time.status.clone(),
                comparison_status: comp.metrics.startup_time.status.clone(),
                duration_ms: metric::integer(
                    startup_duration(base),
                    startup_duration(comp),
                    Direction::LowerIsBetter,
                ),
            },
            memory: MemoryComparison {
                baseline_rss_bytes: integer(base, comp, |run| {
                    run.metrics.memory.as_ref().map(|m| m.baseline.rss_bytes)
                }),
                peak_rss_bytes: integer(base, comp, |run| {
                    run.metrics.memory.as_ref().map(|m| m.peak.rss_bytes)
                }),
                delta_rss_bytes: integer(base, comp, |run| {
                    run.metrics.memory.as_ref().map(|m| m.delta.rss_bytes)
                }),
            },
            cpu: CpuComparison {
                baseline_percent: decimal(base, comp, |run| {
                    run.metrics.cpu.as_ref().map(|c| c.baseline_percent)
                }),
                average_percent: decimal(base, comp, |run| {
                    run.metrics.cpu.as_ref().map(|c| c.average_percent)
                }),
                peak_percent: decimal(base, comp, |run| {
                    run.metrics.cpu.as_ref().map(|c| c.peak_percent)
                }),
            },
            http_latency: HttpLatencyComparison {
                baseline_status: http_status(base),
                comparison_status: http_status(comp),
                requests_per_second: metric::decimal(
                    http_throughput(base),
                    http_throughput(comp),
                    Direction::HigherIsBetter,
                ),
                p50_ms: metric::decimal(
                    http_stat(base, |l| l.p50_ms),
                    http_stat(comp, |l| l.p50_ms),
                    Direction::LowerIsBetter,
                ),
                p95_ms: metric::decimal(
                    http_stat(base, |l| l.p95_ms),
                    http_stat(comp, |l| l.p95_ms),
                    Direction::LowerIsBetter,
                ),
                p99_ms: metric::decimal(
                    http_stat(base, |l| l.p99_ms),
                    http_stat(comp, |l| l.p99_ms),
                    Direction::LowerIsBetter,
                ),
            },
            artifact_size: ArtifactSizeComparison {
                source: category(base, comp, |run| &run.metrics.artifact_size.source),
                build: category(base, comp, |run| &run.metrics.artifact_size.build),
                excluded: category(base, comp, |run| &run.metrics.artifact_size.excluded),
            },
        },
    }
}

fn reference(run: &StoredRun) -> RunReference {
    RunReference {
        run_id: run.result.run_id,
        target: run.result.target.clone(),
        runtime: run.result.discovery.runtime.clone(),
        created_at: run.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    }
}

fn build_duration(run: &BenchmarkResult) -> Option<u64> {
    let build = &run.metrics.build_time;

    if build.status == STATUS_SUCCESS {
        build.duration_ms
    } else {
        None
    }
}

fn startup_duration(run: &BenchmarkResult) -> Option<u64> {
    let startup = &run.metrics.startup_time;

    if startup.status == STATUS_SUCCESS {
        startup.duration_ms
    } else {
        None
    }
}

fn http_status(run: &BenchmarkResult) -> Option<String> {
    run.metrics
        .http_latency
        .as_ref()
        .map(|latency| latency.status.clone())
}

// Only a completed workload is comparable
fn completed_http(run: &BenchmarkResult) -> Option<&HttpLatency> {
    run.metrics
        .http_latency
        .as_ref()
        .filter(|latency| latency.status == STATUS_COMPLETED)
}

// Zero observed requests is "no measurement", not a throughput of zero
fn http_throughput(run: &BenchmarkResult) -> Option<f64> {
    completed_http(run)
        .filter(|latency| latency.total_requests > 0)
        .map(|latency| latency.requests_per_second)
}

fn http_stat(run: &BenchmarkResult, read: impl Fn(&HttpLatency) -> Option<f64>) -> Option<f64> {
    completed_http(run).and_then(read)
}

// Memory and CPU are lower is better
fn integer(
    baseline: &BenchmarkResult,
    comparison: &BenchmarkResult,
    read: impl Fn(&BenchmarkResult) -> Option<u64>,
) -> metric::IntegerMetric {
    metric::integer(read(baseline), read(comparison), Direction::LowerIsBetter)
}

fn decimal(
    baseline: &BenchmarkResult,
    comparison: &BenchmarkResult,
    read: impl Fn(&BenchmarkResult) -> Option<f64>,
) -> metric::DecimalMetric {
    metric::decimal(read(baseline), read(comparison), Direction::LowerIsBetter)
}

// Bytes and file count are both lower is better
fn category(
    baseline: &BenchmarkResult,
    comparison: &BenchmarkResult,
    read: impl Fn(&BenchmarkResult) -> &ArtifactSizeCategory,
) -> ArtifactCategoryComparison {
    let (base, comp) = (read(baseline), read(comparison));

    ArtifactCategoryComparison {
        bytes: metric::integer(Some(base.bytes), Some(comp.bytes), Direction::LowerIsBetter),
        file_count: metric::integer(
            Some(base.file_count),
            Some(comp.file_count),
            Direction::LowerIsBetter,
        ),
    }
}
