pub mod benchmark;

use crate::config::env::Env;
use axum::Router;

pub fn router() -> Router<Env> {
    Router::new().nest("/api/v1", benchmark::handler::router())
}
