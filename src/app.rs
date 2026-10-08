use std::sync::Arc;

use axum::Router;

use crate::{
    app_state::AppState, config::env::Env, errors::fallback, features::v1,
    persistence::repository::BenchmarkRepository,
};

// Full application router, shared by main and the API tests
pub fn router(env: Env, repository: Arc<dyn BenchmarkRepository>) -> Router {
    Router::new()
        .merge(v1::router())
        .fallback(fallback::not_found)
        .with_state(AppState { env, repository })
}
