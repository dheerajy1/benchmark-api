use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row, postgres::PgRow};

use crate::{
    features::v1::benchmark::metrics::http_latency::STATUS_COMPLETED,
    persistence::{
        error::PersistenceError,
        history::{ProjectRow, ProjectTotals, TrendQuery, TrendRow},
    },
};

use super::mapping::STATUS_SUCCESS;

// Every stored run that is not deleted
const COUNT_RUNS: &str = "SELECT COUNT(*) FROM benchmark_runs WHERE deleted_at IS NULL";

// Fixed SQL, the status codes and the limit are only ever bound, never written into the text.
// $1 is the success code, $2 the completed code and $3 the limit. $4 is the project path and
// $5 the run status, a NULL leaves that filter out.
// The comparison rules live in the CASE expressions: a build or startup duration only counts
// when that step succeeded, and the HTTP numbers only count for a completed workload with at
// least one request. Memory and CPU have no status, they are set when their row exists.
// A CASE without a match is NULL, so an unavailable measurement is never turned into zero.
// NUMERIC columns are cast to float8 because sqlx reads f64 from float8 only.
const SELECT_TRENDS: &str = "SELECT r.id, r.created_at, \
     CASE WHEN bs.code = $1 THEN b.duration_ms END AS build_time_ms, \
     CASE WHEN ss.code = $1 THEN s.duration_ms END AS startup_time_ms, \
     m.peak_rss_bytes AS memory_peak_bytes, \
     c.average_percent::float8 AS cpu_average_percent, \
     CASE WHEN hs.code = $2 AND h.total_requests > 0 \
          THEN h.requests_per_second::float8 END AS requests_per_second, \
     CASE WHEN hs.code = $2 AND h.total_requests > 0 \
          THEN h.total_requests END AS total_requests, \
     CASE WHEN hs.code = $2 AND h.total_requests > 0 \
          THEN h.p95_ms::float8 END AS p95_ms \
     FROM benchmark_runs r \
     LEFT JOIN benchmark_builds b ON b.benchmark_run_id = r.id \
     LEFT JOIN benchmark_statuses bs ON bs.id = b.status_id \
     LEFT JOIN benchmark_startups s ON s.benchmark_run_id = r.id \
     LEFT JOIN benchmark_statuses ss ON ss.id = s.status_id \
     LEFT JOIN benchmark_memories m ON m.benchmark_run_id = r.id \
     LEFT JOIN benchmark_cpus c ON c.benchmark_run_id = r.id \
     LEFT JOIN benchmark_http_latencies h ON h.benchmark_run_id = r.id \
     LEFT JOIN benchmark_statuses hs ON hs.id = h.status_id \
     WHERE r.deleted_at IS NULL \
       AND ($4::text IS NULL OR r.target_path = $4) \
       AND ($5::text IS NULL OR r.status = $5) \
     ORDER BY r.created_at DESC, r.id DESC LIMIT $3";

// The complete historical count of persisted runs
pub async fn count_runs(pool: &PgPool) -> Result<i64, PersistenceError> {
    let count = sqlx::query_scalar::<_, i64>(COUNT_RUNS)
        .fetch_one(pool)
        .await?;

    Ok(count)
}

// The newest runs with the comparison rules applied by the database, newest first
pub async fn list_trends(
    pool: &PgPool,
    query: &TrendQuery,
) -> Result<Vec<TrendRow>, PersistenceError> {
    let rows = sqlx::query(SELECT_TRENDS)
        .bind(STATUS_SUCCESS)
        .bind(STATUS_COMPLETED)
        .bind(query.limit)
        .bind(query.target_path.clone())
        .bind(query.status.map(|status| status.as_str().to_string()))
        .fetch_all(pool)
        .await?;

    rows.iter().map(trend_row).collect()
}

// Every project once. The name, runtime and source come from the newest run that has them,
// ordered by (created_at, id) like every other list.
const SELECT_PROJECTS: &str = "SELECT r.target_path, \
     count(*) AS total_runs, \
     count(*) FILTER (WHERE r.status = 'done') AS done_runs, \
     max(r.created_at) AS last_run_at, \
     (array_agg(cfg.target_name ORDER BY r.created_at DESC, r.id DESC))[1] AS target_name, \
     (array_agg(rt.code ORDER BY r.created_at DESC, r.id DESC) \
         FILTER (WHERE rt.code IS NOT NULL))[1] AS runtime_code, \
     (array_agg(cfg.source ORDER BY r.created_at DESC, r.id DESC))[1] AS source \
     FROM benchmark_runs r \
     LEFT JOIN runtimes rt ON rt.id = r.runtime_id \
     LEFT JOIN benchmark_run_configurations cfg ON cfg.run_id = r.id \
     WHERE r.deleted_at IS NULL \
     GROUP BY r.target_path \
     ORDER BY last_run_at DESC, r.target_path";

const SELECT_PROJECT_TOTALS: &str = "SELECT count(*) AS total_runs, \
     count(*) FILTER (WHERE status = 'done') AS done_runs, \
     count(*) FILTER (WHERE status = 'failed') AS failed_runs, \
     count(*) FILTER (WHERE status = 'cancelled') AS cancelled_runs \
     FROM benchmark_runs \
     WHERE deleted_at IS NULL AND target_path = $1";

pub async fn list_projects(pool: &PgPool) -> Result<Vec<ProjectRow>, PersistenceError> {
    let rows = sqlx::query(SELECT_PROJECTS).fetch_all(pool).await?;

    rows.iter()
        .map(|row| {
            Ok(ProjectRow {
                path: row.try_get("target_path")?,
                target_name: row.try_get("target_name")?,
                runtime_code: row.try_get("runtime_code")?,
                source: row.try_get("source")?,
                total_runs: row.try_get("total_runs")?,
                done_runs: row.try_get("done_runs")?,
                last_run_at: row.try_get::<DateTime<Utc>, _>("last_run_at")?,
            })
        })
        .collect()
}

pub async fn project_totals(
    pool: &PgPool,
    target_path: &str,
) -> Result<ProjectTotals, PersistenceError> {
    let row = sqlx::query(SELECT_PROJECT_TOTALS)
        .bind(target_path)
        .fetch_one(pool)
        .await?;

    Ok(ProjectTotals {
        total_runs: row.try_get("total_runs")?,
        done_runs: row.try_get("done_runs")?,
        failed_runs: row.try_get("failed_runs")?,
        cancelled_runs: row.try_get("cancelled_runs")?,
    })
}

fn trend_row(row: &PgRow) -> Result<TrendRow, PersistenceError> {
    Ok(TrendRow {
        id: row.try_get("id")?,
        created_at: row.try_get("created_at")?,
        build_time_ms: row.try_get("build_time_ms")?,
        startup_time_ms: row.try_get("startup_time_ms")?,
        memory_peak_bytes: row.try_get("memory_peak_bytes")?,
        cpu_average_percent: row.try_get("cpu_average_percent")?,
        requests_per_second: row.try_get("requests_per_second")?,
        total_requests: row.try_get("total_requests")?,
        p95_ms: row.try_get("p95_ms")?,
    })
}
