pub mod benchmark;
pub mod comparison;
pub mod health;
pub mod overview;
pub mod status;
pub mod targets;

use crate::app_state::AppState;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        benchmark::handler::router()
            .merge(benchmark::live::handler::router())
            .merge(comparison::handler::router())
            .merge(health::handler::router())
            .merge(overview::handler::router())
            .merge(status::handler::router())
            .merge(targets::handler::router()),
    )
}
