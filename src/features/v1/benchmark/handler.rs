use axum::{Router, extract::State, routing::post};

use crate::{
    config::env::Env,
    errors::{
        api_error::ApiError, catalog::ErrorCode, fallback::method_not_allowed, json::ApiJson,
        response::ApiSuccess,
    },
};

use super::{
    model::{BenchmarkRequest, BenchmarkResult},
    service,
};

async fn benchmark(
    State(env): State<Env>,
    ApiJson(request): ApiJson<BenchmarkRequest>,
) -> Result<ApiSuccess<BenchmarkResult>, ApiError> {
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

    Ok(ApiSuccess::ok("Benchmark completed.", result))
}

pub fn router() -> Router<Env> {
    Router::new().route("/benchmarks", post(benchmark).fallback(method_not_allowed))
}
