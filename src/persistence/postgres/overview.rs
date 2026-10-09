use sqlx::{PgPool, Row, postgres::PgRow};

use crate::{
    features::v1::benchmark::metrics::http_latency::STATUS_COMPLETED,
    persistence::{error::PersistenceError, history::TrendRow},
};

use super::mapping::STATUS_SUCCESS;

// Every stored run, no filter
const COUNT_RUNS: &str = "SELECT COUNT(*) FROM benchmark_runs";

// Fixed SQL, the status codes and the limit are only ever bound, never written into the text.
// $1 is the success code, $2 the completed code and $3 the limit.
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
     ORDER BY r.created_at DESC, r.id DESC LIMIT $3";

// The complete historical count of persisted runs
pub async fn count_runs(pool: &PgPool) -> Result<i64, PersistenceError> {
    let count = sqlx::query_scalar::<_, i64>(COUNT_RUNS)
        .fetch_one(pool)
        .await?;

    Ok(count)
}

// The newest runs with the comparison rules applied by the database, newest first
pub async fn list_trends(pool: &PgPool, limit: i64) -> Result<Vec<TrendRow>, PersistenceError> {
    let rows = sqlx::query(SELECT_TRENDS)
        .bind(STATUS_SUCCESS)
        .bind(STATUS_COMPLETED)
        .bind(limit)
        .fetch_all(pool)
        .await?;

    rows.iter().map(trend_row).collect()
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
        p95_ms: row.try_get("p95_ms")?,
    })
}
