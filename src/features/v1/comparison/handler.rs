use axum::{
    Router,
    extract::{Query, State, rejection::QueryRejection},
    routing::get,
};

use crate::{
    app_state::AppState,
    errors::{
        api_error::ApiError, catalog::ErrorCode, fallback::method_not_allowed, response::ApiSuccess,
    },
};

use super::{model::ComparisonResponse, query::ComparisonQueryParams, service};

// GET /api/v1/comparisons/summary?runIds=<baseline>,<comparison>
async fn summary(
    State(state): State<AppState>,
    query: Result<Query<ComparisonQueryParams>, QueryRejection>,
) -> Result<ApiSuccess<ComparisonResponse>, ApiError> {
    let Query(query) = query
        .map_err(|rejection| ApiError::new(ErrorCode::ValidationError, rejection.body_text()))?;

    let ids = query.validate()?;

    let comparison = service::compare(state.repository.as_ref(), ids).await?;

    Ok(ApiSuccess::ok("Comparison completed.", comparison))
}

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/comparisons/summary",
        get(summary).fallback(method_not_allowed),
    )
}
