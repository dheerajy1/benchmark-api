use serde::Deserialize;

use crate::features::v1::benchmark::redact::REDACTED;

pub const DEFAULT_READY_TIMEOUT_SECONDS: u64 = 60;
pub const DEFAULT_IDLE_SECONDS: u64 = 30;
// Long on purpose, a cold Rust image build takes many minutes
pub const DEFAULT_BUILD_TIMEOUT_SECONDS: u64 = 1800;

// The placeholder in docker.run_command that is replaced with the temporary env file path
pub const ENV_FILE_PLACEHOLDER: &str = "{ENV_FILE}";

// One extra process the run measures after the main project, for example the recordlog
// subscriber, natively and in Docker
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRequest {
    // Label in results, also the key that matches runs when comparing
    pub name: String,
    // Folder of the workspace, relative to the project path
    pub directory: String,
    #[serde(default)]
    pub env: Option<WorkspaceEnv>,
    // Text that must appear in the output before the process counts as ready
    #[serde(default)]
    pub ready_log_line: String,
    #[serde(default = "default_ready_timeout")]
    pub ready_timeout_seconds: u64,
    #[serde(default)]
    pub load: Option<LoadRequest>,
    // Sampling window when there is no load command
    #[serde(default = "default_idle_seconds")]
    pub idle_seconds: u64,
    #[serde(default)]
    pub native: Option<NativeRequest>,
    #[serde(default)]
    pub docker: Option<DockerRequest>,
}

impl WorkspaceRequest {
    pub fn native_enabled(&self) -> bool {
        self.native.as_ref().is_some_and(|native| native.enabled)
    }

    pub fn docker_enabled(&self) -> bool {
        self.docker.as_ref().is_some_and(|docker| docker.enabled)
    }
}

// Either a path to an env file or the pasted body of one, never both
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceEnv {
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub pasted: Option<String>,
}

// The pasted text holds secret values, so it is never printed
impl std::fmt::Debug for WorkspaceEnv {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceEnv")
            .field("file_path", &self.file_path)
            .field("pasted", &self.pasted.as_ref().map(|_| REDACTED))
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadRequest {
    pub command: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRequest {
    #[serde(default)]
    pub enabled: bool,
    // Optional, runs once before the start command, for example a compile step
    #[serde(default)]
    pub build_command: Option<String>,
    #[serde(default)]
    pub command: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockerRequest {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub build_command: String,
    #[serde(default = "default_build_timeout")]
    pub build_timeout_seconds: u64,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub container_name: String,
    // May contain the {ENV_FILE} placeholder
    #[serde(default)]
    pub run_command: String,
}

fn default_ready_timeout() -> u64 {
    DEFAULT_READY_TIMEOUT_SECONDS
}

fn default_idle_seconds() -> u64 {
    DEFAULT_IDLE_SECONDS
}

fn default_build_timeout() -> u64 {
    DEFAULT_BUILD_TIMEOUT_SECONDS
}
