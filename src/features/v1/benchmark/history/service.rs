use chrono::SecondsFormat;
use uuid::Uuid;

use crate::{
    features::v1::benchmark::model::Target,
    persistence::{
        error::PersistenceError,
        history::{CursorPosition, HistoryCounts, SummaryQuery, SummaryRow, WorkspaceSummaryRow},
        outcome::StoredStatus,
        path::display_name,
        repository::BenchmarkRepository,
    },
};

use super::{
    cursor,
    error::HistoryError,
    model::{
        BenchmarkDetail, BenchmarkPage, BenchmarkSummary, Counts, Pagination, StageEntry,
        StatusCounts, SummaryDiscovery, SummaryMetrics, WorkspaceSummary,
    },
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
        status: params.status,
        search: params.search,
    };

    let mut rows = repository
        .list_summaries(&query)
        .await
        .map_err(map_list_error)?;

    let counts = repository
        .count_history(&query)
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
        counts: to_counts(counts)?,
        pagination: Pagination {
            limit,
            has_more,
            next_cursor,
        },
    })
}

// One stored run with how it ended and its stage times, a deleted run does not exist
pub async fn get(
    repository: &dyn BenchmarkRepository,
    run_id: Uuid,
) -> Result<BenchmarkDetail, HistoryError> {
    let stored = repository
        .get_runs(&[run_id])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| HistoryError::NotFound(format!("Benchmark run {run_id} was not found.")))?;

    Ok(BenchmarkDetail {
        display_name: display_name(stored.target_name.as_deref(), &stored.result.target.path),
        status: stored.status.as_str(),
        stopped_stage: stored.stopped_stage,
        failure_message: stored.failure_message,
        stages: stored
            .stages
            .into_iter()
            .map(|stage| StageEntry {
                stage: stage.stage,
                status: stage.status,
                duration_ms: stage.duration_ms,
            })
            .collect(),
        result: stored.result,
    })
}

// Deleting twice is fine, only an id that was never stored is not found
pub async fn delete(
    repository: &dyn BenchmarkRepository,
    run_id: Uuid,
) -> Result<(), HistoryError> {
    if repository.delete_run(run_id).await? {
        Ok(())
    } else {
        Err(HistoryError::NotFound(format!(
            "Benchmark run {run_id} was not found."
        )))
    }
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
        status: row.status.as_str(),
        display_name: display_name(row.target_name.as_deref(), &row.target_path),
        target: Target {
            path: row.target_path,
        },
        discovery: SummaryDiscovery {
            runtime: row.runtime_code,
        },
        created_at: row.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        summary: SummaryMetrics {
            build_output_bytes: non_negative("build_output_bytes", row.build_output_bytes)?,
            workspaces: row
                .workspaces
                .into_iter()
                .map(to_workspace)
                .collect::<Result<Vec<_>, _>>()?,
            build_time_ms: non_negative("build_time_ms", row.build_time_ms)?,
            startup_time_ms: non_negative("startup_time_ms", row.startup_time_ms)?,
            memory_peak_bytes: non_negative("memory_peak_bytes", row.memory_peak_bytes)?,
            cpu_average_percent: row.cpu_average_percent,
            requests_per_second: row.requests_per_second,
            total_requests: non_negative("total_requests", row.total_requests)?,
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

fn to_workspace(row: WorkspaceSummaryRow) -> Result<WorkspaceSummary, HistoryError> {
    Ok(WorkspaceSummary {
        name: row.name,
        mode: row.mode,
        status: row.status,
        image_bytes: non_negative("image_bytes", row.image_bytes)?,
        disk_usage_bytes: non_negative("disk_usage_bytes", row.disk_usage_bytes)?,
        build_duration_ms: non_negative("build_duration_ms", row.build_duration_ms)?,
    })
}

fn to_counts(counts: HistoryCounts) -> Result<Counts, HistoryError> {
    let mut by_status = StatusCounts {
        done: 0,
        failed: 0,
        cancelled: 0,
    };

    for (status, runs) in counts.by_status {
        let runs = count("by_status", runs)?;

        match status {
            StoredStatus::Done => by_status.done = runs,
            StoredStatus::Failed => by_status.failed = runs,
            StoredStatus::Cancelled => by_status.cancelled = runs,
        }
    }

    Ok(Counts {
        total: count("total", counts.total)?,
        by_runtime: counts
            .by_runtime
            .into_iter()
            .map(|(code, runs)| Ok((code, count("by_runtime", runs)?)))
            .collect::<Result<_, HistoryError>>()?,
        by_status,
    })
}

fn count(field: &str, value: i64) -> Result<u64, HistoryError> {
    u64::try_from(value).map_err(|_| {
        HistoryError::Persistence(PersistenceError::InvalidResult(format!(
            "{field} count is negative: {value}"
        )))
    })
}
