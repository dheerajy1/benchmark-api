use chrono::SecondsFormat;

use crate::{
    features::v1::benchmark::history::{query::ListParams, service as history_service},
    persistence::{
        history::{ProjectRow, ProjectTotals, SummaryQuery, TrendQuery},
        outcome::StoredStatus,
        path::display_name,
        repository::BenchmarkRepository,
    },
};

use super::{
    convert::{non_negative, to_trend_point},
    error::OverviewError,
    model::{Overview, Project, ProjectEntry, Totals},
};

// How many of the newest done runs the trend holds
pub const TREND_LIMIT: i64 = 30;

// Reads one project, every query behind it filters by the same path so nothing mixes.
// Without a path the project of the newest done run is shown, or of the newest run when no
// run is done. A path without runs is answered with zero totals, not with an error.
pub async fn overview(
    repository: &dyn BenchmarkRepository,
    target_path: Option<String>,
) -> Result<Overview, OverviewError> {
    let projects = repository.list_projects().await?;

    let path = match target_path {
        Some(path) => Some(path),
        None => default_path(repository).await?,
    };

    let Some(path) = path else {
        return Ok(Overview {
            project: None,
            projects: Vec::new(),
            totals: totals(ProjectTotals::default())?,
            latest: None,
            previous: None,
            trends: Vec::new(),
        });
    };

    let project = projects
        .iter()
        .find(|project| project.path == path)
        .map_or_else(
            || Project {
                name: display_name(None, &path),
                path: path.clone(),
                runtime: None,
                source: None,
            },
            to_project,
        );

    let totals = totals(repository.project_totals(&path).await?)?;

    // The newest two done runs of the project, in the exact summary form of GET /benchmarks
    let page = history_service::list(
        repository,
        ListParams {
            limit: 2,
            before: None,
            runtime: None,
            target_path: Some(path.clone()),
            status: Some(StoredStatus::Done),
            search: None,
        },
    )
    .await?;

    let mut done = page.items.into_iter();
    let latest = done.next();
    let previous = done.next();

    let trends = repository
        .list_trends(&TrendQuery {
            limit: TREND_LIMIT,
            target_path: Some(path),
            status: Some(StoredStatus::Done),
        })
        .await?
        .into_iter()
        .map(to_trend_point)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Overview {
        project: Some(project),
        projects: projects
            .into_iter()
            .map(to_entry)
            .collect::<Result<Vec<_>, _>>()?,
        totals,
        latest,
        previous,
        trends,
    })
}

// The path of the newest done run, otherwise of the newest run, None when nothing is stored
async fn default_path(
    repository: &dyn BenchmarkRepository,
) -> Result<Option<String>, OverviewError> {
    for status in [Some(StoredStatus::Done), None] {
        let rows = repository
            .list_summaries(&SummaryQuery {
                limit: 1,
                status,
                ..SummaryQuery::default()
            })
            .await?;

        if let Some(row) = rows.into_iter().next() {
            return Ok(Some(row.target_path));
        }
    }

    Ok(None)
}

fn to_project(row: &ProjectRow) -> Project {
    Project {
        path: row.path.clone(),
        name: display_name(row.target_name.as_deref(), &row.path),
        runtime: row.runtime_code.clone(),
        source: row.source.clone(),
    }
}

fn to_entry(row: ProjectRow) -> Result<ProjectEntry, OverviewError> {
    Ok(ProjectEntry {
        name: display_name(row.target_name.as_deref(), &row.path),
        total_runs: non_negative("total_runs", row.total_runs)?,
        done_runs: non_negative("done_runs", row.done_runs)?,
        last_run_at: row.last_run_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        path: row.path,
        runtime: row.runtime_code,
    })
}

fn totals(totals: ProjectTotals) -> Result<Totals, OverviewError> {
    Ok(Totals {
        total_runs: non_negative("total_runs", totals.total_runs)?,
        done_runs: non_negative("done_runs", totals.done_runs)?,
        failed_runs: non_negative("failed_runs", totals.failed_runs)?,
        cancelled_runs: non_negative("cancelled_runs", totals.cancelled_runs)?,
    })
}
