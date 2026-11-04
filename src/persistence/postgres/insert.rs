use sqlx::{PgConnection, types::Json};
use uuid::Uuid;

use crate::persistence::error::PersistenceError;

use super::{
    lookup::{Lookup, lookup_id},
    rows::{
        ArtifactSizeRow, BenchmarkRows, BuildRow, CpuRow, HttpLatencyRow, MemoryRow, RunRow,
        StageRow, StartupRow, WorkspaceRow,
    },
};

// Inserts one benchmark into every table, the caller owns the transaction
pub async fn insert_all(
    connection: &mut PgConnection,
    rows: &BenchmarkRows,
) -> Result<(), PersistenceError> {
    let run_id = rows.run.id;

    insert_run(connection, &rows.run).await?;

    for row in &rows.artifact_sizes {
        insert_artifact_size(connection, run_id, row).await?;
    }

    if let Some(build) = &rows.build {
        insert_build(connection, run_id, build).await?;
    }

    if let Some(startup) = &rows.startup {
        insert_startup(connection, run_id, startup).await?;
    }

    if let Some(memory) = &rows.memory {
        insert_memory(connection, run_id, memory).await?;
    }

    if let Some(cpu) = &rows.cpu {
        insert_cpu(connection, run_id, cpu).await?;
    }

    if let Some(http_latency) = &rows.http_latency {
        insert_http_latency(connection, run_id, http_latency).await?;
    }

    for workspace in &rows.workspaces {
        insert_workspace(connection, run_id, workspace).await?;
    }

    for stage in &rows.stages {
        insert_stage(connection, run_id, stage.position, stage).await?;
    }

    Ok(())
}

// The saving stage is the time the writes took up to here, it can only be known now. A run
// that already carries a saving row keeps it.
pub async fn insert_saving_stage(
    connection: &mut PgConnection,
    rows: &BenchmarkRows,
    elapsed: std::time::Duration,
) -> Result<(), PersistenceError> {
    if rows.stages.iter().any(|stage| stage.stage == "saving") {
        return Ok(());
    }

    let row = StageRow {
        position: i32::try_from(rows.stages.len()).map_err(|_| {
            PersistenceError::InvalidResult("too many stages in one run".to_string())
        })?,
        stage: "saving".to_string(),
        status: "success".to_string(),
        duration_ms: i64::try_from(elapsed.as_millis()).ok(),
    };

    insert_stage(connection, rows.run.id, row.position, &row).await
}

async fn insert_stage(
    connection: &mut PgConnection,
    run_id: Uuid,
    position: i32,
    row: &StageRow,
) -> Result<(), PersistenceError> {
    sqlx::query(
        "INSERT INTO benchmark_run_stages \
         (id, benchmark_run_id, position, stage, status, duration_ms) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .bind(position)
    .bind(&row.stage)
    .bind(&row.status)
    .bind(row.duration_ms)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

async fn insert_run(connection: &mut PgConnection, row: &RunRow) -> Result<(), PersistenceError> {
    let runtime_id = match &row.runtime_code {
        Some(code) => Some(lookup_id(connection, Lookup::Runtime, code).await?),
        None => None,
    };

    sqlx::query(
        "INSERT INTO benchmark_runs \
         (id, target_path, runtime_id, created_at, status, stopped_stage, failure_message) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(row.id)
    .bind(&row.target_path)
    .bind(runtime_id)
    .bind(row.created_at)
    .bind(row.status.as_str())
    .bind(row.stopped_stage.as_deref())
    .bind(row.failure_message.as_deref())
    .execute(&mut *connection)
    .await?;

    Ok(())
}

async fn insert_artifact_size(
    connection: &mut PgConnection,
    run_id: Uuid,
    row: &ArtifactSizeRow,
) -> Result<(), PersistenceError> {
    let artifact_type_id =
        lookup_id(connection, Lookup::ArtifactType, row.artifact_type_code).await?;

    sqlx::query(
        "INSERT INTO artifact_sizes (id, benchmark_run_id, artifact_type_id, bytes, file_count) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .bind(artifact_type_id)
    .bind(row.bytes)
    .bind(row.file_count)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

async fn insert_build(
    connection: &mut PgConnection,
    run_id: Uuid,
    row: &BuildRow,
) -> Result<(), PersistenceError> {
    let status_id = lookup_id(connection, Lookup::Status, &row.status_code).await?;

    sqlx::query(
        "INSERT INTO benchmark_builds \
         (id, benchmark_run_id, status_id, duration_ms, program, args, working_directory, \
          started_at, finished_at, exit_code) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .bind(status_id)
    .bind(row.duration_ms)
    .bind(row.program.as_deref())
    .bind(row.args.as_ref().map(|args| Json(args.clone())))
    .bind(row.working_directory.as_deref())
    .bind(row.started_at)
    .bind(row.finished_at)
    .bind(row.exit_code)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

async fn insert_startup(
    connection: &mut PgConnection,
    run_id: Uuid,
    row: &StartupRow,
) -> Result<(), PersistenceError> {
    let status_id = lookup_id(connection, Lookup::Status, &row.status_code).await?;

    sqlx::query(
        "INSERT INTO benchmark_startups \
         (id, benchmark_run_id, status_id, duration_ms, program, args, working_directory, \
          started_at, ready_at, exit_code, readiness_type, readiness_host, readiness_port, \
          readiness_path, readiness_method, readiness_expected_status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .bind(status_id)
    .bind(row.duration_ms)
    .bind(row.program.as_deref())
    .bind(row.args.as_ref().map(|args| Json(args.clone())))
    .bind(row.working_directory.as_deref())
    .bind(row.started_at)
    .bind(row.ready_at)
    .bind(row.exit_code)
    .bind(row.readiness_type.as_deref())
    .bind(row.readiness_host.as_deref())
    .bind(row.readiness_port)
    .bind(row.readiness_path.as_deref())
    .bind(row.readiness_method.as_deref())
    .bind(row.readiness_expected_status)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

async fn insert_memory(
    connection: &mut PgConnection,
    run_id: Uuid,
    row: &MemoryRow,
) -> Result<(), PersistenceError> {
    sqlx::query(
        "INSERT INTO benchmark_memories \
         (id, benchmark_run_id, baseline_rss_bytes, peak_rss_bytes, delta_rss_bytes) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .bind(row.baseline_rss_bytes)
    .bind(row.peak_rss_bytes)
    .bind(row.delta_rss_bytes)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

// f64 values are cast to float8 in SQL, the NUMERIC column then applies its own scale
async fn insert_cpu(
    connection: &mut PgConnection,
    run_id: Uuid,
    row: &CpuRow,
) -> Result<(), PersistenceError> {
    sqlx::query(
        "INSERT INTO benchmark_cpus \
         (id, benchmark_run_id, baseline_percent, average_percent, peak_percent) \
         VALUES ($1, $2, $3::float8, $4::float8, $5::float8)",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .bind(row.baseline_percent)
    .bind(row.average_percent)
    .bind(row.peak_percent)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

async fn insert_http_latency(
    connection: &mut PgConnection,
    run_id: Uuid,
    row: &HttpLatencyRow,
) -> Result<(), PersistenceError> {
    let status_id = lookup_id(connection, Lookup::Status, &row.status_code).await?;

    sqlx::query(
        "INSERT INTO benchmark_http_latencies \
         (id, benchmark_run_id, status_id, total_requests, successful_requests, \
          failed_requests, duration_ms, requests_per_second, min_ms, mean_ms, p50_ms, \
          p95_ms, p99_ms, max_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::float8, $9::float8, $10::float8, \
                 $11::float8, $12::float8, $13::float8, $14::float8)",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .bind(status_id)
    .bind(row.total_requests)
    .bind(row.successful_requests)
    .bind(row.failed_requests)
    .bind(row.duration_ms)
    .bind(row.requests_per_second)
    .bind(row.min_ms)
    .bind(row.mean_ms)
    .bind(row.p50_ms)
    .bind(row.p95_ms)
    .bind(row.p99_ms)
    .bind(row.max_ms)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

async fn insert_workspace(
    connection: &mut PgConnection,
    run_id: Uuid,
    row: &WorkspaceRow,
) -> Result<(), PersistenceError> {
    let status_id = lookup_id(connection, Lookup::Status, &row.status_code).await?;

    let build_status_id = match &row.build_status_code {
        Some(code) => Some(lookup_id(connection, Lookup::Status, code).await?),
        None => None,
    };

    let load_status_id = match &row.load_status_code {
        Some(code) => Some(lookup_id(connection, Lookup::Status, code).await?),
        None => None,
    };

    let memory = row.memory.as_ref();
    let cpu = row.cpu.as_ref();

    sqlx::query(
        "INSERT INTO benchmark_workspace_results \
         (id, benchmark_run_id, workspace_name, mode, status_id, message, env_variable_names, \
          build_status_id, build_duration_ms, build_exit_code, image_bytes, startup_ms, \
          baseline_rss_bytes, peak_rss_bytes, delta_rss_bytes, \
          cpu_baseline_percent, cpu_average_percent, cpu_peak_percent, \
          load_status_id, load_exit_code, load_duration_ms, idle_seconds, image_disk_usage_bytes) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, \
                 $16::float8, $17::float8, $18::float8, $19, $20, $21, $22, $23)",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .bind(&row.name)
    .bind(&row.mode)
    .bind(status_id)
    .bind(row.message.as_deref())
    .bind(Json(row.env_variable_names.clone()))
    .bind(build_status_id)
    .bind(row.build_duration_ms)
    .bind(row.build_exit_code)
    .bind(row.image_bytes)
    .bind(row.startup_ms)
    .bind(memory.map(|memory| memory.baseline_rss_bytes))
    .bind(memory.map(|memory| memory.peak_rss_bytes))
    .bind(memory.map(|memory| memory.delta_rss_bytes))
    .bind(cpu.map(|cpu| cpu.baseline_percent))
    .bind(cpu.map(|cpu| cpu.average_percent))
    .bind(cpu.map(|cpu| cpu.peak_percent))
    .bind(load_status_id)
    .bind(row.load_exit_code)
    .bind(row.load_duration_ms)
    .bind(row.idle_seconds)
    .bind(row.image_disk_usage_bytes)
    .execute(&mut *connection)
    .await?;

    Ok(())
}
