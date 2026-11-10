use std::collections::HashMap;

use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder, Row, postgres::PgRow};
use uuid::Uuid;

use crate::persistence::{
    error::PersistenceError,
    history::{HistoryCounts, SummaryQuery, SummaryRow, WorkspaceSummaryRow},
    outcome::StoredStatus,
};

use super::lookup::{Lookup, lookup_id};

// The key of a run without a detected runtime in the runtime counts
pub const UNKNOWN_RUNTIME: &str = "unknown";

// Fixed SQL, user supplied values are only ever bound, never written into the text.
// NUMERIC columns are cast to float8 because sqlx reads f64 from float8 only.
const SELECT_SUMMARIES: &str = "SELECT r.id, r.target_path, cfg.target_name, \
     rt.code AS runtime_code, r.status AS run_status, r.created_at, \
     a.bytes AS build_output_bytes, \
     b.duration_ms AS build_time_ms, \
     s.duration_ms AS startup_time_ms, \
     m.peak_rss_bytes AS memory_peak_bytes, \
     c.average_percent::float8 AS cpu_average_percent, \
     h.requests_per_second::float8 AS requests_per_second, \
     h.total_requests AS total_requests, \
     h.p95_ms::float8 AS p95_ms \
     FROM benchmark_runs r \
     LEFT JOIN runtimes rt ON rt.id = r.runtime_id \
     LEFT JOIN benchmark_run_configurations cfg ON cfg.run_id = r.id \
     LEFT JOIN artifact_sizes a ON a.benchmark_run_id = r.id \
         AND a.artifact_type_id = (SELECT id FROM artifact_types WHERE code = 'build') \
     LEFT JOIN benchmark_builds b ON b.benchmark_run_id = r.id \
     LEFT JOIN benchmark_startups s ON s.benchmark_run_id = r.id \
     LEFT JOIN benchmark_memories m ON m.benchmark_run_id = r.id \
     LEFT JOIN benchmark_cpus c ON c.benchmark_run_id = r.id \
     LEFT JOIN benchmark_http_latencies h ON h.benchmark_run_id = r.id";

const SELECT_WORKSPACE_SUMMARIES: &str = "SELECT w.benchmark_run_id, w.workspace_name, w.mode, \
     s.code AS status_code, w.image_bytes, w.image_disk_usage_bytes, w.build_duration_ms \
     FROM benchmark_workspace_results w \
     JOIN benchmark_statuses s ON s.id = w.status_id \
     WHERE w.benchmark_run_id = ANY($1) \
     ORDER BY w.id";

// The filters of a query with the runtime code already resolved to its id
struct Filters<'a> {
    runtime_id: Option<Uuid>,
    query: &'a SummaryQuery,
}

// Which filters a statement leaves out, the counts of a facet ignore the facet's own filter
#[derive(Clone, Copy, PartialEq, Eq)]
enum Skip {
    Nothing,
    Runtime,
    Status,
}

// A deleted run is never visible, then every active filter. Always starts the WHERE clause.
fn push_filters(builder: &mut QueryBuilder<'_, Postgres>, filters: &Filters<'_>, skip: Skip) {
    builder.push(" WHERE r.deleted_at IS NULL");

    if skip != Skip::Runtime
        && let Some(runtime_id) = filters.runtime_id
    {
        builder.push(" AND r.runtime_id = ").push_bind(runtime_id);
    }

    if skip != Skip::Status
        && let Some(status) = filters.query.status
    {
        builder
            .push(" AND r.status = ")
            .push_bind(status.as_str().to_string());
    }

    if let Some(target_path) = &filters.query.target_path {
        builder
            .push(" AND r.target_path = ")
            .push_bind(target_path.clone());
    }

    if let Some(search) = &filters.query.search {
        let pattern = like_pattern(search);

        builder
            .push(" AND (r.target_path ILIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE '\\' OR r.id::text ILIKE ")
            .push_bind(pattern)
            .push(" ESCAPE '\\')");
    }
}

// The text is matched as typed, so the LIKE wildcards in it are escaped
fn like_pattern(search: &str) -> String {
    let mut pattern = String::with_capacity(search.len() + 2);

    pattern.push('%');

    for character in search.chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }

        pattern.push(character);
    }

    pattern.push('%');

    pattern
}

// An unknown runtime code is reported, not silently treated as an empty result
async fn resolve<'a>(
    connection: &mut PgConnection,
    query: &'a SummaryQuery,
) -> Result<Filters<'a>, PersistenceError> {
    let runtime_id = match &query.runtime_code {
        Some(code) => Some(lookup_id(connection, Lookup::Runtime, code).await?),
        None => None,
    };

    Ok(Filters { runtime_id, query })
}

// Filters first, then the (created_at, id) cursor, then ordering and the limit, all in SQL
pub async fn list(
    pool: &PgPool,
    query: &SummaryQuery,
) -> Result<Vec<SummaryRow>, PersistenceError> {
    let mut connection = pool.acquire().await?;
    let filters = resolve(&mut connection, query).await?;

    let mut builder = QueryBuilder::<Postgres>::new(SELECT_SUMMARIES);

    push_filters(&mut builder, &filters, Skip::Nothing);

    if let Some(after) = &query.after {
        builder
            .push(" AND (r.created_at, r.id) < (")
            .push_bind(after.created_at)
            .push(", ")
            .push_bind(after.id)
            .push(")");
    }

    builder
        .push(" ORDER BY r.created_at DESC, r.id DESC LIMIT ")
        .push_bind(query.limit);

    let rows = builder.build().fetch_all(&mut *connection).await?;

    let ids: Vec<Uuid> = rows
        .iter()
        .map(|row| row.try_get("id"))
        .collect::<Result<_, _>>()?;

    let mut workspaces = fetch_workspaces(&mut connection, &ids).await?;

    rows.iter()
        .map(|row| {
            let id: Uuid = row.try_get("id")?;

            summary_row(row, workspaces.remove(&id).unwrap_or_default())
        })
        .collect()
}

// The pill numbers, three statements that share the same filter code
pub async fn count(pool: &PgPool, query: &SummaryQuery) -> Result<HistoryCounts, PersistenceError> {
    let mut connection = pool.acquire().await?;
    let filters = resolve(&mut connection, query).await?;

    let mut total = QueryBuilder::<Postgres>::new("SELECT count(*) FROM benchmark_runs r");

    push_filters(&mut total, &filters, Skip::Nothing);

    let total: i64 = total
        .build_query_scalar()
        .fetch_one(&mut *connection)
        .await?;

    let mut by_runtime = QueryBuilder::<Postgres>::new(
        "SELECT COALESCE(rt.code, 'unknown') AS facet, count(*) AS runs \
         FROM benchmark_runs r LEFT JOIN runtimes rt ON rt.id = r.runtime_id",
    );

    push_filters(&mut by_runtime, &filters, Skip::Runtime);

    by_runtime.push(" GROUP BY facet ORDER BY facet");

    let by_runtime = by_runtime
        .build()
        .fetch_all(&mut *connection)
        .await?
        .iter()
        .map(|row| Ok((row.try_get("facet")?, row.try_get("runs")?)))
        .collect::<Result<Vec<(String, i64)>, PersistenceError>>()?;

    let mut by_status = QueryBuilder::<Postgres>::new(
        "SELECT r.status AS facet, count(*) AS runs FROM benchmark_runs r",
    );

    push_filters(&mut by_status, &filters, Skip::Status);

    by_status.push(" GROUP BY facet");

    let found = by_status
        .build()
        .fetch_all(&mut *connection)
        .await?
        .iter()
        .map(|row| {
            let code: String = row.try_get("facet")?;
            let runs: i64 = row.try_get("runs")?;

            Ok((code, runs))
        })
        .collect::<Result<Vec<(String, i64)>, PersistenceError>>()?;

    let by_status = [
        StoredStatus::Done,
        StoredStatus::Failed,
        StoredStatus::Cancelled,
    ]
    .into_iter()
    .map(|status| {
        let runs = found
            .iter()
            .find(|(code, _)| code == status.as_str())
            .map_or(0, |(_, runs)| *runs);

        (status, runs)
    })
    .collect();

    Ok(HistoryCounts {
        total,
        by_runtime,
        by_status,
    })
}

async fn fetch_workspaces(
    connection: &mut PgConnection,
    run_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<WorkspaceSummaryRow>>, PersistenceError> {
    let rows = sqlx::query(SELECT_WORKSPACE_SUMMARIES)
        .bind(run_ids)
        .fetch_all(&mut *connection)
        .await?;

    let mut grouped: HashMap<Uuid, Vec<WorkspaceSummaryRow>> = HashMap::new();

    for row in &rows {
        grouped
            .entry(row.try_get("benchmark_run_id")?)
            .or_default()
            .push(WorkspaceSummaryRow {
                name: row.try_get("workspace_name")?,
                mode: row.try_get("mode")?,
                status: row.try_get("status_code")?,
                image_bytes: row.try_get("image_bytes")?,
                disk_usage_bytes: row.try_get("image_disk_usage_bytes")?,
                build_duration_ms: row.try_get("build_duration_ms")?,
            });
    }

    Ok(grouped)
}

fn summary_row(
    row: &PgRow,
    workspaces: Vec<WorkspaceSummaryRow>,
) -> Result<SummaryRow, PersistenceError> {
    let status: String = row.try_get("run_status")?;

    Ok(SummaryRow {
        id: row.try_get("id")?,
        target_path: row.try_get("target_path")?,
        target_name: row.try_get("target_name")?,
        runtime_code: row.try_get("runtime_code")?,
        status: StoredStatus::from_code(&status).ok_or_else(|| {
            PersistenceError::InvalidResult(format!("unknown run status '{status}'"))
        })?,
        created_at: row.try_get("created_at")?,
        build_output_bytes: row.try_get("build_output_bytes")?,
        workspaces,
        build_time_ms: row.try_get("build_time_ms")?,
        startup_time_ms: row.try_get("startup_time_ms")?,
        memory_peak_bytes: row.try_get("memory_peak_bytes")?,
        cpu_average_percent: row.try_get("cpu_average_percent")?,
        requests_per_second: row.try_get("requests_per_second")?,
        total_requests: row.try_get("total_requests")?,
        p95_ms: row.try_get("p95_ms")?,
    })
}
