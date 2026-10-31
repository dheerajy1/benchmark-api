use serde::Serialize;

use crate::features::v1::benchmark::model::{Cpu, Memory};

pub const MODE_NATIVE: &str = "native";
pub const MODE_DOCKER: &str = "docker";

// "success" or a reason the mode did not produce numbers
pub const STATUS_SUCCESS: &str = "success";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_TIMEOUT: &str = "timeout";

// What one mode of one workspace measured. A mode that could not be measured keeps its status
// and null numbers, nothing is invented.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceResult {
    pub name: String,
    // "native" or "docker"
    pub mode: String,
    pub status: String,
    // Why the mode failed, never holds an environment variable value
    pub message: Option<String>,
    // Names only, values are never stored or returned
    pub env_variable_names: Vec<String>,
    pub build: Option<WorkspaceBuild>,
    pub image: Option<ImageSize>,
    pub startup: Option<WorkspaceStartup>,
    pub memory: Option<Memory>,
    pub cpu: Option<Cpu>,
    pub load: Option<LoadResult>,
    // Sampling window used when no load command ran
    pub idle_seconds: Option<u64>,
}

impl WorkspaceResult {
    pub fn empty(name: &str, mode: &str, env_variable_names: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            mode: mode.to_string(),
            status: STATUS_FAILED.to_string(),
            message: None,
            env_variable_names,
            build: None,
            image: None,
            startup: None,
            memory: None,
            cpu: None,
            load: None,
            idle_seconds: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceBuild {
    // "success", "failed" or "timeout"
    pub status: String,
    pub duration_ms: u64,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImageSize {
    pub bytes: u64,
    pub size: String,
    // Unpacked size on disk as Docker lists it, null when this Docker does not report it
    pub disk_usage_bytes: Option<u64>,
    pub disk_usage: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceStartup {
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoadResult {
    // "success" for exit code 0, "failed" otherwise
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
}
