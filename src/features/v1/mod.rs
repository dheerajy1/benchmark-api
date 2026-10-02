pub mod benchmark;

use axum::Router;

pub fn router() -> Router {
    Router::new().nest("/api/v1", benchmark::handler::router())
}
