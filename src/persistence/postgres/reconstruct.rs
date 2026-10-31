use chrono::{DateTime, SecondsFormat, Utc};

use crate::{
    features::v1::benchmark::{
        metrics::artifact_size::format_size,
        model::{
            ArtifactSize, ArtifactSizeCategory, BenchmarkResult, BuildCommand, BuildTime, Cpu,
            Discovery, HttpLatency, Memory, MemoryReading, Metrics, StartupReadiness, StartupTime,
            Target,
        },
        workspace::result::{
            ImageSize, LoadResult, WorkspaceBuild, WorkspaceResult, WorkspaceStartup,
        },
    },
    persistence::error::PersistenceError,
};

use super::{
    mapping::{ARTIFACT_BUILD, ARTIFACT_EXCLUDED, ARTIFACT_SOURCE},
    rows::{BuildRow, CpuRow, HttpLatencyRow, MemoryRow, RunRow, StartupRow, WorkspaceRow},
};

// One artifact_sizes row joined with its artifact type code
pub struct StoredArtifactSize {
    pub artifact_type_code: String,
    pub bytes: i64,
    pub file_count: i64,
}

// Everything stored for one run, read back from the normalized tables
pub struct StoredBenchmark {
    pub run: RunRow,
    pub artifact_sizes: Vec<StoredArtifactSize>,
    pub build: BuildRow,
    pub startup: StartupRow,
    pub memory: Option<MemoryRow>,
    pub cpu: Option<CpuRow>,
    pub http_latency: Option<HttpLatencyRow>,
    pub workspaces: Vec<WorkspaceRow>,
}

// Rebuilds the exact BenchmarkResult the POST endpoint returned, formatted strings included
pub fn to_result(stored: StoredBenchmark) -> Result<BenchmarkResult, PersistenceError> {
    Ok(BenchmarkResult {
        run_id: stored.run.id,
        target: Target {
            path: stored.run.target_path,
        },
        // The build discovery is never stored, the POST result always reports null
        discovery: Discovery {
            runtime: stored.run.runtime_code,
            build: None,
        },
        metrics: Metrics {
            artifact_size: rebuild_artifact_size(&stored.artifact_sizes)?,
            build_time: rebuild_build(stored.build)?,
            startup_time: rebuild_startup(stored.startup)?,
            memory: stored.memory.map(rebuild_memory).transpose()?,
            cpu: stored.cpu.map(rebuild_cpu),
            http_latency: stored.http_latency.map(rebuild_http_latency).transpose()?,
            workspaces: stored
                .workspaces
                .into_iter()
                .map(rebuild_workspace)
                .collect::<Result<Vec<_>, _>>()?,
        },
    })
}

fn to_u64(field: &str, value: i64) -> Result<u64, PersistenceError> {
    u64::try_from(value).map_err(|_| {
        PersistenceError::InvalidResult(format!("{field} is negative in the database: {value}"))
    })
}

fn optional_u64(field: &str, value: Option<i64>) -> Result<Option<u64>, PersistenceError> {
    value.map(|value| to_u64(field, value)).transpose()
}

// Same format the measurement code produces: RFC 3339, UTC, milliseconds
fn format_timestamp(value: Option<DateTime<Utc>>) -> Option<String> {
    value.map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn command(program: Option<String>, args: Option<Vec<String>>) -> Option<BuildCommand> {
    program.map(|program| BuildCommand {
        program,
        args: args.unwrap_or_default(),
    })
}

fn rebuild_artifact_size(sizes: &[StoredArtifactSize]) -> Result<ArtifactSize, PersistenceError> {
    Ok(ArtifactSize {
        source: rebuild_category(sizes, ARTIFACT_SOURCE)?,
        build: rebuild_category(sizes, ARTIFACT_BUILD)?,
        excluded: rebuild_category(sizes, ARTIFACT_EXCLUDED)?,
    })
}

fn rebuild_category(
    sizes: &[StoredArtifactSize],
    code: &str,
) -> Result<ArtifactSizeCategory, PersistenceError> {
    let stored = sizes
        .iter()
        .find(|stored| stored.artifact_type_code == code)
        .ok_or_else(|| {
            PersistenceError::InvalidResult(format!("artifact size '{code}' is missing"))
        })?;

    let bytes = to_u64("artifact_size.bytes", stored.bytes)?;

    Ok(ArtifactSizeCategory {
        bytes,
        size: format_size(bytes),
        file_count: to_u64("artifact_size.file_count", stored.file_count)?,
    })
}

fn rebuild_build(row: BuildRow) -> Result<BuildTime, PersistenceError> {
    Ok(BuildTime {
        status: row.status_code,
        duration_ms: optional_u64("build_time.duration_ms", row.duration_ms)?,
        command: command(row.program, row.args),
        working_directory: row.working_directory,
        started_at: format_timestamp(row.started_at),
        finished_at: format_timestamp(row.finished_at),
        exit_code: row.exit_code,
    })
}

fn rebuild_startup(row: StartupRow) -> Result<StartupTime, PersistenceError> {
    // Built first, it borrows the row before fields are moved out of it
    let readiness = rebuild_readiness(&row)?;

    Ok(StartupTime {
        status: row.status_code,
        duration_ms: optional_u64("startup_time.duration_ms", row.duration_ms)?,
        command: command(row.program, row.args),
        working_directory: row.working_directory,
        readiness,
        started_at: format_timestamp(row.started_at),
        ready_at: format_timestamp(row.ready_at),
        exit_code: row.exit_code,
    })
}

// The schema keeps the six readiness columns all set or all NULL
fn rebuild_readiness(row: &StartupRow) -> Result<Option<StartupReadiness>, PersistenceError> {
    let (Some(readiness_type), Some(host), Some(port), Some(path), Some(method), Some(status)) = (
        &row.readiness_type,
        &row.readiness_host,
        row.readiness_port,
        &row.readiness_path,
        &row.readiness_method,
        row.readiness_expected_status,
    ) else {
        return Ok(None);
    };

    Ok(Some(StartupReadiness {
        readiness_type: readiness_type.clone(),
        host: host.clone(),
        port: u16::try_from(port).map_err(|_| {
            PersistenceError::InvalidResult(format!("readiness port is out of range: {port}"))
        })?,
        path: path.clone(),
        method: method.clone(),
        expected_status: u16::try_from(status).map_err(|_| {
            PersistenceError::InvalidResult(format!(
                "readiness expected status is out of range: {status}"
            ))
        })?,
    }))
}

fn rebuild_workspace(row: WorkspaceRow) -> Result<WorkspaceResult, PersistenceError> {
    let build = match (row.build_status_code, row.build_duration_ms) {
        (Some(status), Some(duration_ms)) => Some(WorkspaceBuild {
            status,
            duration_ms: to_u64("workspace.build.duration_ms", duration_ms)?,
            exit_code: row.build_exit_code,
        }),
        _ => None,
    };

    let load = match (row.load_status_code, row.load_duration_ms) {
        (Some(status), Some(duration_ms)) => Some(LoadResult {
            status,
            exit_code: row.load_exit_code,
            duration_ms: to_u64("workspace.load.duration_ms", duration_ms)?,
        }),
        _ => None,
    };

    let disk_usage_bytes = optional_u64(
        "workspace.image.disk_usage_bytes",
        row.image_disk_usage_bytes,
    )?;

    let image = optional_u64("workspace.image.bytes", row.image_bytes)?.map(|bytes| ImageSize {
        bytes,
        size: format_size(bytes),
        disk_usage_bytes,
        disk_usage: disk_usage_bytes.map(format_size),
    });

    let startup = optional_u64("workspace.startup.duration_ms", row.startup_ms)?
        .map(|duration_ms| WorkspaceStartup { duration_ms });

    Ok(WorkspaceResult {
        name: row.name,
        mode: row.mode,
        status: row.status_code,
        message: row.message,
        env_variable_names: row.env_variable_names,
        build,
        image,
        startup,
        memory: row.memory.map(rebuild_memory).transpose()?,
        cpu: row.cpu.map(rebuild_cpu),
        load,
        idle_seconds: optional_u64("workspace.idle_seconds", row.idle_seconds)?,
    })
}

fn rebuild_memory(row: MemoryRow) -> Result<Memory, PersistenceError> {
    Ok(Memory {
        baseline: reading("memory.baseline.rss_bytes", row.baseline_rss_bytes)?,
        peak: reading("memory.peak.rss_bytes", row.peak_rss_bytes)?,
        delta: reading("memory.delta.rss_bytes", row.delta_rss_bytes)?,
    })
}

fn reading(field: &str, value: i64) -> Result<MemoryReading, PersistenceError> {
    let rss_bytes = to_u64(field, value)?;

    Ok(MemoryReading {
        rss_bytes,
        rss_size: format_size(rss_bytes),
    })
}

fn rebuild_cpu(row: CpuRow) -> Cpu {
    Cpu {
        baseline_percent: row.baseline_percent,
        average_percent: row.average_percent,
        peak_percent: row.peak_percent,
    }
}

fn rebuild_http_latency(row: HttpLatencyRow) -> Result<HttpLatency, PersistenceError> {
    Ok(HttpLatency {
        status: row.status_code,
        total_requests: to_u64("http_latency.total_requests", row.total_requests)?,
        successful_requests: to_u64("http_latency.successful_requests", row.successful_requests)?,
        failed_requests: to_u64("http_latency.failed_requests", row.failed_requests)?,
        duration_ms: to_u64("http_latency.duration_ms", row.duration_ms)?,
        requests_per_second: row.requests_per_second,
        min_ms: row.min_ms,
        mean_ms: row.mean_ms,
        p50_ms: row.p50_ms,
        p95_ms: row.p95_ms,
        p99_ms: row.p99_ms,
        max_ms: row.max_ms,
    })
}
