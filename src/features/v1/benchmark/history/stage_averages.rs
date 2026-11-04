use std::collections::BTreeMap;

use axum::extract::{Query, State, rejection::QueryRejection};
use serde::{Deserialize, Serialize};

use crate::{
    app_state::AppState,
    errors::{api_error::ApiError, catalog::ErrorCode, response::ApiSuccess},
    persistence::{
        error::PersistenceError, history::StageRun, path::trim_trailing_slashes,
        repository::BenchmarkRepository,
    },
};

// How many of the newest done runs of a target the averages look at
pub const RUNS_USED_MAX: i64 = 3;

#[derive(Debug, Default, Deserialize)]
pub struct StageAveragesParams {
    target_path: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct StageAverages {
    // How many done runs were read, 0 to 3
    pub runs_used: u64,
    // Run order, a stage that only some runs have is averaged over the runs that have it
    pub stages: Vec<StageAverage>,
    // The average of the totals of the runs, null when no run has a stage duration
    pub total_average_ms: Option<u64>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct StageAverage {
    pub stage: String,
    pub average_ms: u64,
    // The duration in the newest run, null when that run has no duration for the stage
    pub last_ms: Option<u64>,
}

// Averages are rounded to the nearest millisecond, half up
fn rounded_average(sum: u128, count: u128) -> u64 {
    ((sum * 2 + count) / (count * 2)) as u64
}

fn duration(value: i64) -> Result<u64, PersistenceError> {
    u64::try_from(value).map_err(|_| {
        PersistenceError::InvalidResult(format!(
            "stage duration is negative in the database: {value}"
        ))
    })
}

// Pure calculation over the runs, newest run first. A missing duration is skipped and never
// read as a zero.
pub fn average(runs: &[StageRun]) -> Result<StageAverages, PersistenceError> {
    // stage -> (position of its first appearance, durations from the newest run backwards)
    let mut per_stage: BTreeMap<&str, (i32, Vec<Option<u64>>)> = BTreeMap::new();
    let mut run_totals: Vec<u128> = Vec::new();

    for (index, run) in runs.iter().enumerate() {
        let mut total: Option<u128> = None;

        for stage in &run.stages {
            let value = stage.duration_ms.map(duration).transpose()?;

            let entry = per_stage
                .entry(stage.stage.as_str())
                .or_insert((stage.position, Vec::new()));

            entry.0 = entry.0.min(stage.position);
            // The run index is the length, so pad the runs this stage was missing from
            while entry.1.len() < index {
                entry.1.push(None);
            }
            entry.1.push(value);

            if let Some(value) = value {
                total = Some(total.unwrap_or(0) + u128::from(value));
            }
        }

        if let Some(total) = total {
            run_totals.push(total);
        }
    }

    let mut stages: Vec<(i32, &str, u64, Option<u64>)> = per_stage
        .into_iter()
        .filter_map(|(stage, (position, durations))| {
            let known: Vec<u64> = durations.iter().flatten().copied().collect();

            if known.is_empty() {
                return None;
            }

            let sum: u128 = known.iter().map(|value| u128::from(*value)).sum();
            let last = durations.first().copied().flatten();

            Some((
                position,
                stage,
                rounded_average(sum, known.len() as u128),
                last,
            ))
        })
        .collect();

    stages.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));

    let total_average_ms = if run_totals.is_empty() {
        None
    } else {
        Some(rounded_average(
            run_totals.iter().sum(),
            run_totals.len() as u128,
        ))
    };

    Ok(StageAverages {
        runs_used: runs.len() as u64,
        stages: stages
            .into_iter()
            .map(|(_, stage, average_ms, last_ms)| StageAverage {
                stage: stage.to_string(),
                average_ms,
                last_ms,
            })
            .collect(),
        total_average_ms,
    })
}

pub async fn stage_averages(
    repository: &dyn BenchmarkRepository,
    target_path: &str,
) -> Result<StageAverages, PersistenceError> {
    let runs = repository
        .recent_stage_runs(target_path, RUNS_USED_MAX)
        .await?;

    average(&runs)
}

// GET /api/v1/benchmarks/stage-averages?target_path=<path>
pub async fn handler(
    State(state): State<AppState>,
    query: Result<Query<StageAveragesParams>, QueryRejection>,
) -> Result<ApiSuccess<StageAverages>, ApiError> {
    let Query(params) = query
        .map_err(|rejection| ApiError::new(ErrorCode::ValidationError, rejection.body_text()))?;

    let path = match params.target_path.as_deref() {
        None => {
            return Err(ApiError::new(
                ErrorCode::ValidationError,
                "target_path is required.",
            ));
        }
        Some("") => {
            return Err(ApiError::new(
                ErrorCode::ValidationError,
                "target_path must not be empty.",
            ));
        }
        Some(path) => trim_trailing_slashes(path).to_string(),
    };

    let averages = stage_averages(state.repository.as_ref(), &path)
        .await
        .map_err(|error| {
            ApiError::new(
                ErrorCode::InternalServerError,
                format!("Failed to read the stage averages: {error}"),
            )
        })?;

    Ok(ApiSuccess::ok("Stage averages retrieved.", averages))
}
