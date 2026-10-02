use axum::{Json, Router, http::StatusCode, routing::post};

use super::{
    model::{BenchmarkRequest, BenchmarkResult},
    service,
};

async fn benchmark(
    Json(request): Json<BenchmarkRequest>,
) -> Result<Json<BenchmarkResult>, (StatusCode, String)> {
    service::run(&request)
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}

pub fn router() -> Router {
    Router::new().route("/benchmarks", post(benchmark))
}
