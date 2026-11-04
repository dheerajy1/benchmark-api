use axum::extract::{Path, Query, State, rejection::QueryRejection};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    errors::{api_error::ApiError, catalog::ErrorCode, response::ApiSuccess},
};

use super::{
    model::{BenchmarkDetail, BenchmarkPage},
    query::{ListParams, ListQueryParams},
    service,
};

// GET /api/v1/benchmarks
pub async fn list(
    State(state): State<AppState>,
    query: Result<Query<ListQueryParams>, QueryRejection>,
) -> Result<ApiSuccess<BenchmarkPage>, ApiError> {
    let Query(query) = query
        .map_err(|rejection| ApiError::new(ErrorCode::ValidationError, rejection.body_text()))?;

    let params: ListParams = query.validate()?;

    let page = service::list(state.repository.as_ref(), params).await?;

    Ok(ApiSuccess::ok("Benchmarks retrieved.", page))
}

fn parse_run_id(run_id: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(run_id).map_err(|_| {
        ApiError::new(
            ErrorCode::BadRequest,
            format!("'{run_id}' is not a valid run id."),
        )
    })
}

// DELETE /api/v1/benchmarks/{run_id}
pub async fn delete(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<ApiSuccess<()>, ApiError> {
    let run_id = parse_run_id(&run_id)?;

    service::delete(state.repository.as_ref(), run_id).await?;

    Ok(ApiSuccess::ok("Benchmark deleted.", ()))
}

// GET /api/v1/benchmarks/{run_id}
pub async fn detail(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<ApiSuccess<BenchmarkDetail>, ApiError> {
    let run_id = parse_run_id(&run_id)?;

    let result = service::get(state.repository.as_ref(), run_id).await?;

    Ok(ApiSuccess::ok("Benchmark retrieved.", result))
}
