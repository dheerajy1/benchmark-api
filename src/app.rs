use axum::Router;

use crate::{config::env::Env, errors::fallback, features::v1};

// Full application router, shared by main and the API tests
pub fn router(env: Env) -> Router {
    Router::new()
        .merge(v1::router())
        .fallback(fallback::not_found)
        .with_state(env)
}
