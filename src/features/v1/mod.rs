pub mod benchmark;
pub mod comparison;
pub mod overview;
pub mod targets;

use crate::app_state::AppState;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        benchmark::handler::router()
            .merge(benchmark::live::handler::router())
            .merge(comparison::handler::router())
            .merge(overview::handler::router())
            .merge(targets::handler::router()),
    )
}
