use sqlx::{PgPool, Postgres, QueryBuilder, Row, postgres::PgRow};

use crate::persistence::{
    error::PersistenceError,
    history::{SummaryQuery, SummaryRow},
};

use super::lookup::{Lookup, lookup_id};

// Fixed SQL, user supplied values are only ever bound, never written into the text.
// NUMERIC columns are cast to float8 because sqlx reads f64 from float8 only.
const SELECT_SUMMARIES: &str = "SELECT r.id, r.target_path, rt.code AS runtime_code, \
     r.created_at, \
     b.duration_ms AS build_time_ms, \
     s.duration_ms AS startup_time_ms, \
     m.peak_rss_bytes AS memory_peak_bytes, \
     c.average_percent::float8 AS cpu_average_percent, \
     h.requests_per_second::float8 AS requests_per_second, \
     h.p95_ms::float8 AS p95_ms \
     FROM benchmark_runs r \
     LEFT JOIN runtimes rt ON rt.id = r.runtime_id \
     LEFT JOIN benchmark_builds b ON b.benchmark_run_id = r.id \
     LEFT JOIN benchmark_startups s ON s.benchmark_run_id = r.id \
     LEFT JOIN benchmark_memories m ON m.benchmark_run_id = r.id \
     LEFT JOIN benchmark_cpus c ON c.benchmark_run_id = r.id \
     LEFT JOIN benchmark_http_latencies h ON h.benchmark_run_id = r.id";

// Filters first, then the (created_at, id) cursor, then ordering and the limit, all in SQL
pub async fn list(
    pool: &PgPool,
    query: &SummaryQuery,
) -> Result<Vec<SummaryRow>, PersistenceError> {
    let mut connection = pool.acquire().await?;

    // An unknown runtime code is reported, not silently treated as an empty result
    let runtime_id = match &query.runtime_code {
        Some(code) => Some(lookup_id(&mut connection, Lookup::Runtime, code).await?),
        None => None,
    };

    let mut builder = QueryBuilder::<Postgres>::new(SELECT_SUMMARIES);
    let mut keyword = " WHERE ";

    if let Some(runtime_id) = runtime_id {
        builder
            .push(keyword)
            .push("r.runtime_id = ")
            .push_bind(runtime_id);
        keyword = " AND ";
    }

    if let Some(target_path) = &query.target_path {
        builder
            .push(keyword)
            .push("r.target_path = ")
            .push_bind(target_path.clone());
        keyword = " AND ";
    }

    if let Some(after) = &query.after {
        builder
            .push(keyword)
            .push("(r.created_at, r.id) < (")
            .push_bind(after.created_at)
            .push(", ")
            .push_bind(after.id)
            .push(")");
    }

    builder
        .push(" ORDER BY r.created_at DESC, r.id DESC LIMIT ")
        .push_bind(query.limit);

    let rows = builder.build().fetch_all(&mut *connection).await?;

    rows.iter().map(summary_row).collect()
}

fn summary_row(row: &PgRow) -> Result<SummaryRow, PersistenceError> {
    Ok(SummaryRow {
        id: row.try_get("id")?,
        target_path: row.try_get("target_path")?,
        runtime_code: row.try_get("runtime_code")?,
        created_at: row.try_get("created_at")?,
        build_time_ms: row.try_get("build_time_ms")?,
        startup_time_ms: row.try_get("startup_time_ms")?,
        memory_peak_bytes: row.try_get("memory_peak_bytes")?,
        cpu_average_percent: row.try_get("cpu_average_percent")?,
        requests_per_second: row.try_get("requests_per_second")?,
        p95_ms: row.try_get("p95_ms")?,
    })
}
