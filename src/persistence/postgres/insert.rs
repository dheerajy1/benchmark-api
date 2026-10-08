use sqlx::{PgConnection, types::Json};
use uuid::Uuid;

use crate::persistence::error::PersistenceError;

use super::{
    lookup::{Lookup, lookup_id},
    rows::{
        ArtifactSizeRow, BenchmarkRows, BuildRow, CpuRow, HttpLatencyRow, MemoryRow, RunRow,
        StartupRow,
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

    insert_build(connection, run_id, &rows.build).await?;
    insert_startup(connection, run_id, &rows.startup).await?;

    if let Some(memory) = &rows.memory {
        insert_memory(connection, run_id, memory).await?;
    }

    if let Some(cpu) = &rows.cpu {
        insert_cpu(connection, run_id, cpu).await?;
    }

    if let Some(http_latency) = &rows.http_latency {
        insert_http_latency(connection, run_id, http_latency).await?;
    }

    Ok(())
}

async fn insert_run(connection: &mut PgConnection, row: &RunRow) -> Result<(), PersistenceError> {
    let runtime_id = match &row.runtime_code {
        Some(code) => Some(lookup_id(connection, Lookup::Runtime, code).await?),
        None => None,
    };

    sqlx::query(
        "INSERT INTO benchmark_runs (id, target_path, runtime_id, created_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(row.id)
    .bind(&row.target_path)
    .bind(runtime_id)
    .bind(row.created_at)
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
