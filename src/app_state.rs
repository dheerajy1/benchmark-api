use std::sync::Arc;

use crate::{
    config::env::Env,
    features::v1::{benchmark::live::LiveRuns, targets::policy::RunPolicy},
    persistence::repository::BenchmarkRepository,
};

// Shared state of every handler: settings, the persistence boundary, the execution
// slot shared by the legacy and the live submission path, and the run policy
#[derive(Clone)]
pub struct AppState {
    pub env: Env,
    pub repository: Arc<dyn BenchmarkRepository>,
    pub live: LiveRuns,
    pub policy: Arc<RunPolicy>,
}
