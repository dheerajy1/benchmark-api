use std::collections::BTreeMap;

use serde::Deserialize;

use super::model::BuildCommand;

// Explicit startup configuration supplied by the caller
#[derive(Debug, Clone, Deserialize)]
pub struct StartupRequest {
    pub command: BuildCommand,
    pub readiness: ReadinessRequest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReadinessRequest {
    #[serde(rename = "type")]
    pub readiness_type: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub method: String,
    pub expected_status: u16,
    // When omitted the readiness probe uses the .env client credentials
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
}

// HTTP workload executed against the running target
#[derive(Debug, Clone, Deserialize)]
pub struct WorkloadRequest {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub requests: u64,
    pub concurrency: u64,
}
