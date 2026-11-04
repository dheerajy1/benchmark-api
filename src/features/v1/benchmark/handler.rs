use axum::{Router, routing::get};

use crate::{app_state::AppState, errors::fallback::method_not_allowed};

use super::history;

// A benchmark is started with POST /api/v1/live-runs only, POST /benchmarks answers 405
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/benchmarks",
            get(history::handler::list).fallback(method_not_allowed),
        )
        .route(
            "/benchmarks/configurations",
            get(history::config::list).fallback(method_not_allowed),
        )
        .route(
            "/benchmarks/stage-averages",
            get(history::stage_averages::handler).fallback(method_not_allowed),
        )
        .route(
            "/benchmarks/{run_id}",
            get(history::handler::detail)
                .delete(history::handler::delete)
                .fallback(method_not_allowed),
        )
        .route(
            "/benchmarks/{run_id}/config",
            get(history::config::detail).fallback(method_not_allowed),
        )
}
