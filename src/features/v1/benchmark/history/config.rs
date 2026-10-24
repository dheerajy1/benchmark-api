use axum::extract::{Path, Query, State, rejection::QueryRejection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    errors::{api_error::ApiError, catalog::ErrorCode, response::ApiSuccess},
    features::v1::benchmark::configuration::{RunConfiguration, StartupSummary},
    persistence::{
        error::PersistenceError,
        repository::{ConfigurationLookup, StoredConfigurationRun},
    },
};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

#[derive(Debug, Deserialize)]
pub struct ListConfigParams {
    pub limit: Option<i64>,
}

// GET /api/v1/benchmarks/{run_id}/config
#[derive(Debug, Serialize)]
pub struct ConfigurationDetail {
    pub run_id: String,
    pub recorded: bool,
    pub configuration: Option<RunConfiguration>,
}

// One entry of GET /api/v1/benchmarks/configurations. path and startup are present for
// every run; configuration is null for runs stored before configurations were recorded.
#[derive(Debug, Serialize)]
pub struct ConfigurationListItem {
    pub run_id: String,
    pub created_at: String,
    pub path: String,
    pub startup: Option<StartupSummary>,
    pub recorded: bool,
    pub configuration: Option<RunConfiguration>,
}

impl From<StoredConfigurationRun> for ConfigurationListItem {
    fn from(run: StoredConfigurationRun) -> Self {
        Self {
            run_id: run.run_id.to_string(),
            created_at: run.created_at.to_rfc3339(),
            path: run.path,
            startup: run.startup,
            recorded: run.configuration.is_some(),
            configuration: run.configuration,
        }
    }
}

// GET /api/v1/benchmarks/{run_id}/config
pub async fn detail(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<ApiSuccess<ConfigurationDetail>, ApiError> {
    let parsed = Uuid::parse_str(&run_id).map_err(|_| {
        ApiError::new(
            ErrorCode::BadRequest,
            format!("'{run_id}' is not a valid run id."),
        )
    })?;

    let lookup = state
        .repository
        .get_configuration(parsed)
        .await
        .map_err(read_failed)?;

    let detail = match lookup {
        ConfigurationLookup::RunMissing => {
            return Err(ApiError::new(
                ErrorCode::NotFound,
                format!("Benchmark run {parsed} was not found."),
            ));
        }
        ConfigurationLookup::NotRecorded => ConfigurationDetail {
            run_id: parsed.to_string(),
            recorded: false,
            configuration: None,
        },
        ConfigurationLookup::Recorded(configuration) => ConfigurationDetail {
            run_id: parsed.to_string(),
            recorded: true,
            configuration: Some(configuration),
        },
    };

    Ok(ApiSuccess::ok("Run configuration retrieved.", detail))
}

// GET /api/v1/benchmarks/configurations?limit=
pub async fn list(
    State(state): State<AppState>,
    query: Result<Query<ListConfigParams>, QueryRejection>,
) -> Result<ApiSuccess<Vec<ConfigurationListItem>>, ApiError> {
    let Query(params) = query
        .map_err(|rejection| ApiError::new(ErrorCode::ValidationError, rejection.body_text()))?;

    let limit = params.limit.unwrap_or(DEFAULT_LIMIT);

    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::new(
            ErrorCode::ValidationError,
            format!("limit must be between 1 and {MAX_LIMIT}."),
        ));
    }

    let runs = state
        .repository
        .list_configurations(limit)
        .await
        .map_err(read_failed)?;

    let items: Vec<ConfigurationListItem> =
        runs.into_iter().map(ConfigurationListItem::from).collect();

    Ok(ApiSuccess::ok("Run configurations retrieved.", items))
}

fn read_failed(error: PersistenceError) -> ApiError {
    ApiError::new(
        ErrorCode::InternalServerError,
        format!("Failed to read the run configuration: {error}"),
    )
}
