use crate::{
    features::v1::benchmark::history::{
        model::BenchmarkSummary, query::ListParams, service as history_service,
    },
    persistence::repository::BenchmarkRepository,
};

use super::{
    convert::{non_negative, to_trend_point},
    error::OverviewError,
    model::{Overview, Totals},
};

// How many of the newest runs the trend holds, fixed because the endpoint has no parameters
pub const TREND_LIMIT: i64 = 30;

// Reads the three parts one after another, nothing is stored and nothing is cached
pub async fn overview(repository: &dyn BenchmarkRepository) -> Result<Overview, OverviewError> {
    let total_runs = non_negative("total_runs", repository.count_runs().await?)?;

    let latest = latest(repository).await?;

    let trends = repository
        .list_trends(TREND_LIMIT)
        .await?
        .into_iter()
        .map(to_trend_point)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Overview {
        totals: Totals { total_runs },
        latest,
        trends,
    })
}

// The newest run in the exact summary form of GET /benchmarks, the history service is the
// single source of that representation. No run means None, never an empty object.
async fn latest(
    repository: &dyn BenchmarkRepository,
) -> Result<Option<BenchmarkSummary>, OverviewError> {
    let params = ListParams {
        limit: 1,
        before: None,
        runtime: None,
        target_path: None,
    };

    let page = history_service::list(repository, params).await?;

    Ok(page.items.into_iter().next())
}
