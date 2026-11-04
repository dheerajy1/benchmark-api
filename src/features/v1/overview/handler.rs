use axum::{
    Router,
    extract::{Query, State, rejection::QueryRejection},
    routing::get,
};
use serde::Deserialize;

use crate::{
    app_state::AppState,
    errors::{api_error::ApiError, fallback::method_not_allowed, response::ApiSuccess},
    persistence::path::trim_trailing_slashes,
};

use super::{error::OverviewError, model::Overview, service};

// The project to show, other parameters are ignored
#[derive(Debug, Default, Deserialize)]
struct OverviewParams {
    target_path: Option<String>,
}

// GET /api/v1/benchmarks/overview?target_path=<path>
async fn overview(
    State(state): State<AppState>,
    query: Result<Query<OverviewParams>, QueryRejection>,
) -> Result<ApiSuccess<Overview>, ApiError> {
    let Query(params) =
        query.map_err(|rejection| OverviewError::Validation(rejection.body_text()))?;

    let target_path = match params.target_path {
        Some(path) if path.is_empty() => {
            return Err(
                OverviewError::Validation("target_path must not be empty.".to_string()).into(),
            );
        }
        // A pinned target always matches its runs, however the path is spelled
        other => other.map(|path| trim_trailing_slashes(&path).to_string()),
    };

    let overview = service::overview(state.repository.as_ref(), target_path).await?;

    Ok(ApiSuccess::ok("Benchmark overview retrieved.", overview))
}

// A static path, which takes priority over the {run_id} route of the history detail
pub fn router() -> Router<AppState> {
    Router::new().route(
        "/benchmarks/overview",
        get(overview).fallback(method_not_allowed),
    )
}
