use axum::{Router, extract::State, routing::get};

use crate::{
    app_state::AppState,
    errors::{api_error::ApiError, fallback::method_not_allowed, response::ApiSuccess},
};

use super::{model::Overview, service};

// GET /api/v1/benchmarks/overview, no query parameters, any that are sent are ignored
async fn overview(State(state): State<AppState>) -> Result<ApiSuccess<Overview>, ApiError> {
    let overview = service::overview(state.repository.as_ref()).await?;

    Ok(ApiSuccess::ok("Benchmark overview retrieved.", overview))
}

// A static path, which takes priority over the {run_id} route of the history detail
pub fn router() -> Router<AppState> {
    Router::new().route(
        "/benchmarks/overview",
        get(overview).fallback(method_not_allowed),
    )
}
