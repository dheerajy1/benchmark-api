pub mod benchmark;
pub mod comparison;
pub mod overview;

use crate::app_state::AppState;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        benchmark::handler::router()
            .merge(comparison::handler::router())
            .merge(overview::handler::router()),
    )
}
