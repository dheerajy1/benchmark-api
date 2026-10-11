use axum::{
    Router,
    extract::State,
    routing::{get, post},
};

use uuid::Uuid;

use crate::{
    app_state::AppState,
    errors::{
        api_error::ApiError, catalog::ErrorCode, fallback::method_not_allowed, json::ApiJson,
        response::ApiSuccess,
    },
};

use super::{
    history,
    live::{RunContext, RunKind, SubmitError},
    model::{BenchmarkRequest, BenchmarkResult},
    service,
};

async fn benchmark(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<BenchmarkRequest>,
) -> Result<ApiSuccess<BenchmarkResult>, ApiError> {
    let env = state.env.clone();

    // Validation comes first and holds nothing, so an invalid request is still a 400
    // while another benchmark is running
    let target = service::prepare(&request)?;

    // The execution slot is shared with the live runs, a second benchmark is refused
    let permit = state
        .live
        .slot()
        .try_acquire(RunKind::Legacy, None)
        .map_err(|active| ApiError::from(SubmitError::Busy(active)))?;

    // The benchmark blocks for a long time and uses reqwest::blocking, so it must
    // run on a blocking thread and not on an async worker.
    //
    // The permit moves into the closure and comes back with the result. If the client
    // disconnects, the handler future is dropped but this closure keeps running, and
    // the permit is only released when the benchmark itself has finished.
    let joined = tokio::task::spawn_blocking(move || {
        let run_id = Uuid::now_v7();
        let ctx = RunContext::noop();
        let result = service::execute(&target, &request, &env, run_id, &ctx, &permit);

        (result, permit)
    })
    .await
    .map_err(|error| {
        ApiError::new(
            ErrorCode::InternalServerError,
            format!("Benchmark task failed: {error}"),
        )
    })?;

    let (result, permit) = joined;
    let result = result?;

    // The result is only reported as completed once it is stored in one transaction
    state.repository.save(&result).await.map_err(|error| {
        ApiError::new(
            ErrorCode::InternalServerError,
            format!("Failed to persist benchmark result: {error}"),
        )
    })?;

    // Released after the result is stored, and on every error path above when the
    // permit goes out of scope
    drop(permit);

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
