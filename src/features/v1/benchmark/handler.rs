use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

use crate::config::env::Env;

use super::{
    model::{BenchmarkRequest, BenchmarkResult},
    service,
};

async fn benchmark(
    State(env): State<Env>,
    Json(request): Json<BenchmarkRequest>,
) -> Result<Json<BenchmarkResult>, (StatusCode, String)> {
    service::run(&request, &env)
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}

pub fn router() -> Router<Env> {
    Router::new().route("/benchmarks", post(benchmark))
}
