use std::collections::HashMap;

use sqlx::{PgConnection, PgPool, Row, postgres::PgRow, types::Json};
use uuid::Uuid;

use crate::{
    features::v1::benchmark::model::BenchmarkResult,
    persistence::{error::PersistenceError, stored_run::StoredRun},
};

use super::{
    reconstruct::{StoredArtifactSize, StoredBenchmark, to_result},
    rows::{BuildRow, CpuRow, HttpLatencyRow, MemoryRow, RunRow, StartupRow, WorkspaceRow},
};

// One row per run with every one-to-one table of the run, the caller adds the WHERE clause. NUMERIC columns are cast to float8.
const SELECT_RUNS: &str = "SELECT r.id, r.target_path, rt.code AS runtime_code, r.created_at, \
     bs.code AS build_status, b.duration_ms AS build_duration_ms, \
     b.program AS build_program, b.args AS build_args, \
     b.working_directory AS build_working_directory, \
     b.started_at AS build_started_at, b.finished_at AS build_finished_at, \
     b.exit_code AS build_exit_code, \
     ss.code AS startup_status, s.duration_ms AS startup_duration_ms, \
     s.program AS startup_program, s.args AS startup_args, \
     s.working_directory AS startup_working_directory, \
     s.started_at AS startup_started_at, s.ready_at AS startup_ready_at, \
     s.exit_code AS startup_exit_code, \
     s.readiness_type, s.readiness_host, s.readiness_port, s.readiness_path, \
     s.readiness_method, s.readiness_expected_status, \
     m.baseline_rss_bytes, m.peak_rss_bytes, m.delta_rss_bytes, \
     c.baseline_percent::float8 AS cpu_baseline_percent, \
     c.average_percent::float8 AS cpu_average_percent, \
     c.peak_percent::float8 AS cpu_peak_percent, \
     hs.code AS http_status, h.total_requests, h.successful_requests, h.failed_requests, \
     h.duration_ms AS http_duration_ms, \
     h.requests_per_second::float8 AS http_requests_per_second, \
     h.min_ms::float8 AS http_min_ms, h.mean_ms::float8 AS http_mean_ms, \
     h.p50_ms::float8 AS http_p50_ms, h.p95_ms::float8 AS http_p95_ms, \
     h.p99_ms::float8 AS http_p99_ms, h.max_ms::float8 AS http_max_ms \
     FROM benchmark_runs r \
     LEFT JOIN runtimes rt ON rt.id = r.runtime_id \
     LEFT JOIN benchmark_builds b ON b.benchmark_run_id = r.id \
     LEFT JOIN benchmark_statuses bs ON bs.id = b.status_id \
     LEFT JOIN benchmark_startups s ON s.benchmark_run_id = r.id \
     LEFT JOIN benchmark_statuses ss ON ss.id = s.status_id \
     LEFT JOIN benchmark_memories m ON m.benchmark_run_id = r.id \
     LEFT JOIN benchmark_cpus c ON c.benchmark_run_id = r.id \
     LEFT JOIN benchmark_http_latencies h ON h.benchmark_run_id = r.id \
     LEFT JOIN benchmark_statuses hs ON hs.id = h.status_id";

const SELECT_ARTIFACT_SIZES: &str = "SELECT a.benchmark_run_id, t.code, a.bytes, a.file_count \
     FROM artifact_sizes a \
     JOIN artifact_types t ON t.id = a.artifact_type_id \
     WHERE a.benchmark_run_id = ANY($1)";

// Workspace results of the requested runs, in the order they were stored (ids are UUIDv7)
const SELECT_WORKSPACES: &str = "SELECT w.benchmark_run_id, w.workspace_name, w.mode, \
     s.code AS status_code, w.message, w.env_variable_names, \
     bs.code AS build_status_code, w.build_duration_ms, w.build_exit_code, \
     w.image_bytes, w.image_disk_usage_bytes, w.startup_ms, \
     w.baseline_rss_bytes, w.peak_rss_bytes, w.delta_rss_bytes, \
     w.cpu_baseline_percent::float8 AS cpu_baseline_percent, \
     w.cpu_average_percent::float8 AS cpu_average_percent, \
     w.cpu_peak_percent::float8 AS cpu_peak_percent, \
     ls.code AS load_status_code, w.load_exit_code, w.load_duration_ms, w.idle_seconds \
     FROM benchmark_workspace_results w \
     JOIN benchmark_statuses s ON s.id = w.status_id \
     LEFT JOIN benchmark_statuses bs ON bs.id = w.build_status_id \
     LEFT JOIN benchmark_statuses ls ON ls.id = w.load_status_id \
     WHERE w.benchmark_run_id = ANY($1) \
     ORDER BY w.id";

// Reads one run from the normalized tables and rebuilds the POST result, None when absent
pub async fn get(pool: &PgPool, run_id: Uuid) -> Result<Option<BenchmarkResult>, PersistenceError> {
    let runs = get_many(pool, &[run_id]).await?;

    Ok(runs.into_iter().next().map(|run| run.result))
}

// Reads the requested runs only, two queries however many ids. Runs that do not exist are
// simply absent from the result, the order of the result is not the order of the ids.
pub async fn get_many(pool: &PgPool, run_ids: &[Uuid]) -> Result<Vec<StoredRun>, PersistenceError> {
    let mut connection = pool.acquire().await?;

    let sql = format!("{SELECT_RUNS} WHERE r.id = ANY($1)");

    let rows = sqlx::query(&sql)
        .bind(run_ids)
        .fetch_all(&mut *connection)
        .await?;

    let mut artifact_sizes = fetch_artifact_sizes(&mut connection, run_ids).await?;
    let mut workspaces = fetch_workspaces(&mut connection, run_ids).await?;

    let mut runs = Vec::with_capacity(rows.len());

    for row in &rows {
        let run_id: Uuid = row.try_get("id")?;
        let sizes = artifact_sizes.remove(&run_id).unwrap_or_default();
        let run_workspaces = workspaces.remove(&run_id).unwrap_or_default();

        let stored = decode(row, sizes, run_workspaces)?;
        let created_at = stored.run.created_at;

        runs.push(StoredRun {
            created_at,
            result: to_result(stored)?,
        });
    }

    Ok(runs)
}

async fn fetch_artifact_sizes(
    connection: &mut PgConnection,
    run_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<StoredArtifactSize>>, PersistenceError> {
    let rows = sqlx::query(SELECT_ARTIFACT_SIZES)
        .bind(run_ids)
        .fetch_all(&mut *connection)
        .await?;

    let mut grouped: HashMap<Uuid, Vec<StoredArtifactSize>> = HashMap::new();

    for row in &rows {
        let run_id: Uuid = row.try_get("benchmark_run_id")?;

        grouped.entry(run_id).or_default().push(StoredArtifactSize {
            artifact_type_code: row.try_get("code")?,
            bytes: row.try_get("bytes")?,
            file_count: row.try_get("file_count")?,
        });
    }

    Ok(grouped)
}

async fn fetch_workspaces(
    connection: &mut PgConnection,
    run_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<WorkspaceRow>>, PersistenceError> {
    let rows = sqlx::query(SELECT_WORKSPACES)
        .bind(run_ids)
        .fetch_all(&mut *connection)
        .await?;

    let mut grouped: HashMap<Uuid, Vec<WorkspaceRow>> = HashMap::new();

    for row in &rows {
        let run_id: Uuid = row.try_get("benchmark_run_id")?;

        grouped
            .entry(run_id)
            .or_default()
            .push(decode_workspace(row)?);
    }

    Ok(grouped)
}

fn decode_workspace(row: &PgRow) -> Result<WorkspaceRow, PersistenceError> {
    let names: Json<Vec<String>> = row.try_get("env_variable_names")?;

    let memory = match row.try_get::<Option<i64>, _>("baseline_rss_bytes")? {
        None => None,
        Some(baseline_rss_bytes) => Some(MemoryRow {
            baseline_rss_bytes,
            peak_rss_bytes: row.try_get("peak_rss_bytes")?,
            delta_rss_bytes: row.try_get("delta_rss_bytes")?,
        }),
    };

    let cpu = match row.try_get::<Option<f64>, _>("cpu_baseline_percent")? {
        None => None,
        Some(baseline_percent) => Some(CpuRow {
            baseline_percent,
            average_percent: row.try_get("cpu_average_percent")?,
            peak_percent: row.try_get("cpu_peak_percent")?,
        }),
    };

    Ok(WorkspaceRow {
        name: row.try_get("workspace_name")?,
        mode: row.try_get("mode")?,
        status_code: row.try_get("status_code")?,
        message: row.try_get("message")?,
        env_variable_names: names.0,
        build_status_code: row.try_get("build_status_code")?,
        build_duration_ms: row.try_get("build_duration_ms")?,
        build_exit_code: row.try_get("build_exit_code")?,
        image_bytes: row.try_get("image_bytes")?,
        image_disk_usage_bytes: row.try_get("image_disk_usage_bytes")?,
        startup_ms: row.try_get("startup_ms")?,
        memory,
        cpu,
        load_status_code: row.try_get("load_status_code")?,
        load_exit_code: row.try_get("load_exit_code")?,
        load_duration_ms: row.try_get("load_duration_ms")?,
        idle_seconds: row.try_get("idle_seconds")?,
    })
}

fn missing_row(table: &str, run_id: Uuid) -> PersistenceError {
    PersistenceError::InvalidResult(format!("run {run_id} has no {table} row"))
}

fn decode(
    row: &PgRow,
    artifact_sizes: Vec<StoredArtifactSize>,
    workspaces: Vec<WorkspaceRow>,
) -> Result<StoredBenchmark, PersistenceError> {
    let run_id: Uuid = row.try_get("id")?;

    let build_status: Option<String> = row.try_get("build_status")?;
    let startup_status: Option<String> = row.try_get("startup_status")?;

    Ok(StoredBenchmark {
        run: RunRow {
            id: run_id,
            target_path: row.try_get("target_path")?,
            runtime_code: row.try_get("runtime_code")?,
            created_at: row.try_get("created_at")?,
        },
        artifact_sizes,
        build: BuildRow {
            status_code: build_status.ok_or_else(|| missing_row("benchmark_builds", run_id))?,
            duration_ms: row.try_get("build_duration_ms")?,
            program: row.try_get("build_program")?,
            args: decode_args(row, "build_args")?,
            working_directory: row.try_get("build_working_directory")?,
            started_at: row.try_get("build_started_at")?,
            finished_at: row.try_get("build_finished_at")?,
            exit_code: row.try_get("build_exit_code")?,
        },
        startup: StartupRow {
            status_code: startup_status.ok_or_else(|| missing_row("benchmark_startups", run_id))?,
            duration_ms: row.try_get("startup_duration_ms")?,
            program: row.try_get("startup_program")?,
            args: decode_args(row, "startup_args")?,
            working_directory: row.try_get("startup_working_directory")?,
            started_at: row.try_get("startup_started_at")?,
            ready_at: row.try_get("startup_ready_at")?,
            exit_code: row.try_get("startup_exit_code")?,
            readiness_type: row.try_get("readiness_type")?,
            readiness_host: row.try_get("readiness_host")?,
            readiness_port: row.try_get("readiness_port")?,
            readiness_path: row.try_get("readiness_path")?,
            readiness_method: row.try_get("readiness_method")?,
            readiness_expected_status: row.try_get("readiness_expected_status")?,
        },
        memory: decode_memory(row)?,
        cpu: decode_cpu(row)?,
        http_latency: decode_http_latency(row)?,
        workspaces,
    })
}

// The argument vector is stored as JSONB
fn decode_args(row: &PgRow, column: &str) -> Result<Option<Vec<String>>, PersistenceError> {
    let value: Option<Json<Vec<String>>> = row.try_get(column)?;

    Ok(value.map(|Json(args)| args))
}

// The memory columns are NOT NULL, so a NULL baseline means the row does not exist
fn decode_memory(row: &PgRow) -> Result<Option<MemoryRow>, PersistenceError> {
    let baseline: Option<i64> = row.try_get("baseline_rss_bytes")?;

    let Some(baseline_rss_bytes) = baseline else {
        return Ok(None);
    };

    Ok(Some(MemoryRow {
        baseline_rss_bytes,
        peak_rss_bytes: row.try_get("peak_rss_bytes")?,
        delta_rss_bytes: row.try_get("delta_rss_bytes")?,
    }))
}

fn decode_cpu(row: &PgRow) -> Result<Option<CpuRow>, PersistenceError> {
    let baseline: Option<f64> = row.try_get("cpu_baseline_percent")?;

    let Some(baseline_percent) = baseline else {
        return Ok(None);
    };

    Ok(Some(CpuRow {
        baseline_percent,
        average_percent: row.try_get("cpu_average_percent")?,
        peak_percent: row.try_get("cpu_peak_percent")?,
    }))
}

// The status is NOT NULL, so a NULL status means the row does not exist
fn decode_http_latency(row: &PgRow) -> Result<Option<HttpLatencyRow>, PersistenceError> {
    let status: Option<String> = row.try_get("http_status")?;

    let Some(status_code) = status else {
        return Ok(None);
    };

    Ok(Some(HttpLatencyRow {
        status_code,
        total_requests: row.try_get("total_requests")?,
        successful_requests: row.try_get("successful_requests")?,
        failed_requests: row.try_get("failed_requests")?,
        duration_ms: row.try_get("http_duration_ms")?,
        requests_per_second: row.try_get("http_requests_per_second")?,
        min_ms: row.try_get("http_min_ms")?,
        mean_ms: row.try_get("http_mean_ms")?,
        p50_ms: row.try_get("http_p50_ms")?,
        p95_ms: row.try_get("http_p95_ms")?,
        p99_ms: row.try_get("http_p99_ms")?,
        max_ms: row.try_get("http_max_ms")?,
    }))
}
