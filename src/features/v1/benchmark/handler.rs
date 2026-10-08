use axum::{
    Router,
    extract::State,
    routing::{get, post},
};

use crate::{
    app_state::AppState,
    errors::{
        api_error::ApiError, catalog::ErrorCode, fallback::method_not_allowed, json::ApiJson,
        response::ApiSuccess,
    },
};

use super::{
    history,
    model::{BenchmarkRequest, BenchmarkResult},
    service,
};

async fn benchmark(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<BenchmarkRequest>,
) -> Result<ApiSuccess<BenchmarkResult>, ApiError> {
    let env = state.env.clone();

    // The benchmark blocks for a long time and uses reqwest::blocking, so it must
    // run on a blocking thread and not on an async worker
    let result = tokio::task::spawn_blocking(move || service::run(&request, &env))
        .await
        .map_err(|error| {
            ApiError::new(
                ErrorCode::InternalServerError,
                format!("Benchmark task failed: {error}"),
            )
        })?;

    let result = result?;

    // The result is only reported as completed once it is stored in one transaction
    state.repository.save(&result).await.map_err(|error| {
        ApiError::new(
            ErrorCode::InternalServerError,
            format!("Failed to persist benchmark result: {error}"),
        )
    })?;

    Ok(ApiSuccess::ok("Benchmark completed.", result))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/benchmarks",
            post(benchmark)
                .get(history::handler::list)
                .fallback(method_not_allowed),
        )
        .route(
            "/benchmarks/{run_id}",
            get(history::handler::detail).fallback(method_not_allowed),
        )
}
