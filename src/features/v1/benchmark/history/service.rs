use chrono::SecondsFormat;
use uuid::Uuid;

use crate::{
    features::v1::benchmark::model::{BenchmarkResult, Target},
    persistence::{
        error::PersistenceError,
        history::{CursorPosition, SummaryQuery, SummaryRow},
        repository::BenchmarkRepository,
    },
};

use super::{
    cursor,
    error::HistoryError,
    model::{BenchmarkPage, BenchmarkSummary, Pagination, SummaryDiscovery, SummaryMetrics},
    query::ListParams,
};

// Lists one page, the repository filters and paginates in SQL
pub async fn list(
    repository: &dyn BenchmarkRepository,
    params: ListParams,
) -> Result<BenchmarkPage, HistoryError> {
    let limit = params.limit;

    // One extra row tells whether another page exists
    let query = SummaryQuery {
        limit: i64::from(limit) + 1,
        after: params.before,
        runtime_code: params.runtime,
        target_path: params.target_path,
    };

    let mut rows = repository
        .list_summaries(&query)
        .await
        .map_err(map_list_error)?;

    let has_more = rows.len() > limit as usize;

    rows.truncate(limit as usize);

    let next_cursor = if has_more {
        rows.last().map(|row| {
            cursor::encode(&CursorPosition {
                created_at: row.created_at,
                id: row.id,
            })
        })
    } else {
        None
    };

    let items = rows
        .into_iter()
        .map(to_summary)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(BenchmarkPage {
        items,
        pagination: Pagination {
            limit,
            has_more,
            next_cursor,
        },
    })
}

// Returns the same data the POST endpoint returned for this run
pub async fn get(
    repository: &dyn BenchmarkRepository,
    run_id: Uuid,
) -> Result<BenchmarkResult, HistoryError> {
    repository
        .get_by_run_id(run_id)
        .await?
        .ok_or_else(|| HistoryError::NotFound(format!("Benchmark run {run_id} was not found.")))
}

// An unseeded runtime code in the filter is a bad request, not a server failure
fn map_list_error(error: PersistenceError) -> HistoryError {
    match error {
        PersistenceError::UnknownLookup {
            table: "runtimes",
            code,
        } => HistoryError::Validation(format!("Unknown runtime '{code}'.")),
        other => HistoryError::Persistence(other),
    }
}

fn to_summary(row: SummaryRow) -> Result<BenchmarkSummary, HistoryError> {
    Ok(BenchmarkSummary {
        run_id: row.id,
        target: Target {
            path: row.target_path,
        },
        discovery: SummaryDiscovery {
            runtime: row.runtime_code,
        },
        created_at: row.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        summary: SummaryMetrics {
            build_time_ms: non_negative("build_time_ms", row.build_time_ms)?,
            startup_time_ms: non_negative("startup_time_ms", row.startup_time_ms)?,
            memory_peak_bytes: non_negative("memory_peak_bytes", row.memory_peak_bytes)?,
            cpu_average_percent: row.cpu_average_percent,
            requests_per_second: row.requests_per_second,
            p95_ms: row.p95_ms,
        },
    })
}

fn non_negative(field: &str, value: Option<i64>) -> Result<Option<u64>, HistoryError> {
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| {
                HistoryError::Persistence(PersistenceError::InvalidResult(format!(
                    "{field} is negative in the database: {value}"
                )))
            })
        })
        .transpose()
}
