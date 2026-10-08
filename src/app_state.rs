use std::sync::Arc;

use crate::{config::env::Env, persistence::repository::BenchmarkRepository};

// Shared state of every handler: settings plus the persistence boundary
#[derive(Clone)]
pub struct AppState {
    pub env: Env,
    pub repository: Arc<dyn BenchmarkRepository>,
}
