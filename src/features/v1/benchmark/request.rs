use std::collections::BTreeMap;

use serde::Deserialize;

use super::model::BuildCommand;

// Explicit startup configuration supplied by the caller
#[derive(Debug, Clone, Deserialize)]
pub struct StartupRequest {
    pub command: BuildCommand,
    pub readiness: ReadinessRequest,
}

#[derive(Clone, Deserialize)]
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

// Header values are never printed, only their names
impl std::fmt::Debug for ReadinessRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadinessRequest")
            .field("readiness_type", &self.readiness_type)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("path", &self.path)
            .field("method", &self.method)
            .field("expected_status", &self.expected_status)
            .field(
                "headers",
                &self.headers.as_ref().map(super::redact::map_names),
            )
            .finish()
    }
}

// HTTP workload executed against the running target, either a fixed request
// count or a sustained duration, exactly one of requests and duration_seconds
#[derive(Clone, Deserialize)]
pub struct WorkloadRequest {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub requests: Option<u64>,
    #[serde(default)]
    pub duration_seconds: Option<u64>,
    pub concurrency: u64,
}

// Header values are never printed, only their names
impl std::fmt::Debug for WorkloadRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkloadRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("headers", &super::redact::map_names(&self.headers))
            .field("requests", &self.requests)
            .field("duration_seconds", &self.duration_seconds)
            .field("concurrency", &self.concurrency)
            .finish()
    }
}
