use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    features::v1::benchmark::model::{
        ArtifactSize, ArtifactSizeCategory, BenchmarkResult, BuildTime, Cpu, HttpLatency, Memory,
        StartupTime,
    },
    persistence::error::PersistenceError,
};

use super::rows::{
    ArtifactSizeRow, BenchmarkRows, BuildRow, CpuRow, HttpLatencyRow, MemoryRow, RunRow, StartupRow,
};

pub const ARTIFACT_SOURCE: &str = "source";
pub const ARTIFACT_BUILD: &str = "build";
pub const ARTIFACT_EXCLUDED: &str = "excluded";

// The status code of a build or startup that succeeded, shared with the overview query
pub const STATUS_SUCCESS: &str = "success";

// Maps the benchmark domain result into database rows, without touching the database
pub fn map_result(result: &BenchmarkResult) -> Result<BenchmarkRows, PersistenceError> {
    Ok(BenchmarkRows {
        run: RunRow {
            id: result.run_id,
            target_path: result.target.path.clone(),
            runtime_code: result.discovery.runtime.clone(),
            created_at: run_created_at(result.run_id)?,
        },
        artifact_sizes: map_artifact_sizes(&result.metrics.artifact_size)?,
        build: map_build(&result.metrics.build_time)?,
        startup: map_startup(&result.metrics.startup_time)?,
        memory: result.metrics.memory.as_ref().map(map_memory).transpose()?,
        cpu: result.metrics.cpu.as_ref().map(map_cpu),
        http_latency: result
            .metrics
            .http_latency
            .as_ref()
            .map(map_http_latency)
            .transpose()?,
    })
}

// u64 domain values must fit the signed BIGINT, never wrap silently
pub fn to_i64(field: &str, value: u64) -> Result<i64, PersistenceError> {
    i64::try_from(value).map_err(|_| {
        PersistenceError::InvalidResult(format!("{field} does not fit in BIGINT: {value}"))
    })
}

// The domain reports timestamps as RFC 3339 strings, the schema stores TIMESTAMPTZ
pub fn parse_timestamp(
    field: &str,
    value: Option<&str>,
) -> Result<Option<DateTime<Utc>>, PersistenceError> {
    let Some(value) = value else {
        return Ok(None);
    };

    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| Some(timestamp.with_timezone(&Utc)))
        .map_err(|error| {
            PersistenceError::InvalidResult(format!(
                "{field} is not a valid RFC 3339 timestamp: {error}"
            ))
        })
}

// created_at is the timestamp embedded in the UUIDv7 run id, roughly when the run started
pub fn run_created_at(run_id: Uuid) -> Result<DateTime<Utc>, PersistenceError> {
    let (seconds, nanoseconds) = run_id
        .get_timestamp()
        .ok_or_else(|| PersistenceError::InvalidResult("run_id is not a UUIDv7".to_string()))?
        .to_unix();

    DateTime::<Utc>::from_timestamp(seconds as i64, nanoseconds).ok_or_else(|| {
        PersistenceError::InvalidResult("run_id timestamp is out of range".to_string())
    })
}

fn map_artifact_sizes(sizes: &ArtifactSize) -> Result<Vec<ArtifactSizeRow>, PersistenceError> {
    Ok(vec![
        map_artifact_size(ARTIFACT_SOURCE, &sizes.source)?,
        map_artifact_size(ARTIFACT_BUILD, &sizes.build)?,
        map_artifact_size(ARTIFACT_EXCLUDED, &sizes.excluded)?,
    ])
}

fn map_artifact_size(
    artifact_type_code: &'static str,
    category: &ArtifactSizeCategory,
) -> Result<ArtifactSizeRow, PersistenceError> {
    Ok(ArtifactSizeRow {
        artifact_type_code,
        bytes: to_i64("artifact_size.bytes", category.bytes)?,
        file_count: to_i64("artifact_size.file_count", category.file_count)?,
    })
}

fn map_build(build: &BuildTime) -> Result<BuildRow, PersistenceError> {
    Ok(BuildRow {
        status_code: build.status.clone(),
        duration_ms: build
            .duration_ms
            .map(|value| to_i64("build_time.duration_ms", value))
            .transpose()?,
        program: build
            .command
            .as_ref()
            .map(|command| command.program.clone()),
        args: build.command.as_ref().map(|command| command.args.clone()),
        working_directory: build.working_directory.clone(),
        started_at: parse_timestamp("build_time.started_at", build.started_at.as_deref())?,
        finished_at: parse_timestamp("build_time.finished_at", build.finished_at.as_deref())?,
        exit_code: build.exit_code,
    })
}

fn map_startup(startup: &StartupTime) -> Result<StartupRow, PersistenceError> {
    let ready_at = parse_timestamp("startup_time.ready_at", startup.ready_at.as_deref())?;

    // A successful startup is only successful because the target became ready
    if startup.status == STATUS_SUCCESS && ready_at.is_none() {
        return Err(PersistenceError::InvalidResult(
            "startup_time has status success but no ready_at".to_string(),
        ));
    }

    let readiness = startup.readiness.as_ref();

    Ok(StartupRow {
        status_code: startup.status.clone(),
        duration_ms: startup
            .duration_ms
            .map(|value| to_i64("startup_time.duration_ms", value))
            .transpose()?,
        program: startup
            .command
            .as_ref()
            .map(|command| command.program.clone()),
        args: startup.command.as_ref().map(|command| command.args.clone()),
        working_directory: startup.working_directory.clone(),
        started_at: parse_timestamp("startup_time.started_at", startup.started_at.as_deref())?,
        ready_at,
        exit_code: startup.exit_code,
        readiness_type: readiness.map(|value| value.readiness_type.clone()),
        readiness_host: readiness.map(|value| value.host.clone()),
        readiness_port: readiness.map(|value| i32::from(value.port)),
        readiness_path: readiness.map(|value| value.path.clone()),
        readiness_method: readiness.map(|value| value.method.clone()),
        readiness_expected_status: readiness.map(|value| i32::from(value.expected_status)),
    })
}

fn map_memory(memory: &Memory) -> Result<MemoryRow, PersistenceError> {
    Ok(MemoryRow {
        baseline_rss_bytes: to_i64("memory.baseline.rss_bytes", memory.baseline.rss_bytes)?,
        peak_rss_bytes: to_i64("memory.peak.rss_bytes", memory.peak.rss_bytes)?,
        delta_rss_bytes: to_i64("memory.delta.rss_bytes", memory.delta.rss_bytes)?,
    })
}

fn map_cpu(cpu: &Cpu) -> CpuRow {
    CpuRow {
        baseline_percent: cpu.baseline_percent,
        average_percent: cpu.average_percent,
        peak_percent: cpu.peak_percent,
    }
}

fn map_http_latency(latency: &HttpLatency) -> Result<HttpLatencyRow, PersistenceError> {
    Ok(HttpLatencyRow {
        status_code: latency.status.clone(),
        total_requests: to_i64("http_latency.total_requests", latency.total_requests)?,
        successful_requests: to_i64(
            "http_latency.successful_requests",
            latency.successful_requests,
        )?,
        failed_requests: to_i64("http_latency.failed_requests", latency.failed_requests)?,
        duration_ms: to_i64("http_latency.duration_ms", latency.duration_ms)?,
        requests_per_second: latency.requests_per_second,
        min_ms: latency.min_ms,
        mean_ms: latency.mean_ms,
        p50_ms: latency.p50_ms,
        p95_ms: latency.p95_ms,
        p99_ms: latency.p99_ms,
        max_ms: latency.max_ms,
    })
}
