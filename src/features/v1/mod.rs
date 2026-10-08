pub mod benchmark;

use crate::app_state::AppState;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new().nest("/api/v1", benchmark::handler::router())
}
