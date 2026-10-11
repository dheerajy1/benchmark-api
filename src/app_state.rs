use std::sync::Arc;

use crate::{
    config::env::Env, features::v1::benchmark::live::LiveRuns,
    persistence::repository::BenchmarkRepository,
};

// Shared state of every handler: settings, the persistence boundary and the execution
// slot shared by the legacy and the live submission path
#[derive(Clone)]
pub struct AppState {
    pub env: Env,
    pub repository: Arc<dyn BenchmarkRepository>,
    pub live: LiveRuns,
}
