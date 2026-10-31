use serde::{Deserialize, Serialize};

use super::{
    model::BenchmarkRequest,
    run_origin::RunOrigin,
    workspace::{config::WorkspaceRequest, env_file},
};

// The configuration a run executed with. Header names only, never a header value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunConfiguration {
    // "custom" for a raw request, "target" for a saved target
    pub source: &'static str,
    pub target_id: Option<String>,
    pub target_name: Option<String>,
    pub target_path: String,
    pub startup: Option<StartupConfiguration>,
    pub workload: Option<WorkloadConfiguration>,
    // Empty for a run without workspaces
    pub workspaces: Vec<WorkspaceConfiguration>,
}

// One workspace as a run used it. Environment variable names only, never a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfiguration {
    pub name: String,
    pub directory: String,
    // "file" or "pasted", None when the workspace had no environment
    pub env_source: Option<String>,
    pub env_file_path: Option<String>,
    // Known here for a pasted env, a file's names are in the workspace results
    pub env_variable_names: Vec<String>,
    pub ready_log_line: String,
    pub ready_timeout_seconds: u64,
    pub load_command: Option<String>,
    pub idle_seconds: u64,
    pub native: Option<NativeConfiguration>,
    pub docker: Option<DockerConfiguration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeConfiguration {
    pub build_command: Option<String>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerConfiguration {
    pub build_command: String,
    pub build_timeout_seconds: u64,
    pub image: String,
    pub container_name: String,
    // The template, the {ENV_FILE} placeholder is still in it
    pub run_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupConfiguration {
    pub program: String,
    pub args: Vec<String>,
    pub readiness: ReadinessConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadinessConfiguration {
    pub readiness_type: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub method: String,
    pub expected_status: u16,
    // None when the server's health credentials are sent
    pub header_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkloadConfiguration {
    pub method: String,
    pub path: String,
    // "requests" or "duration"
    pub mode: &'static str,
    pub requests: Option<u64>,
    pub duration_seconds: Option<u64>,
    pub concurrency: u64,
    pub header_names: Vec<String>,
}

// The start command and readiness as a configuration-list entry shows them. Runs stored
// before configurations were recorded have only what the existing tables hold, so every
// part can be absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupSummary {
    pub program: Option<String>,
    pub args: Option<Vec<String>>,
    pub readiness: Option<ReadinessSummary>,
}

// Readiness settings of a list entry. header_names is null when the header names were not
// recorded, which is the case for every run stored before configurations were recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadinessSummary {
    pub readiness_type: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub method: String,
    pub expected_status: u16,
    pub header_names: Option<Vec<String>>,
}

impl RunConfiguration {
    // Built from the request the run executes, which validation has already accepted
    pub fn from_request(request: &BenchmarkRequest, origin: &RunOrigin) -> Self {
        let (source, target_id, target_name) = match origin {
            RunOrigin::Custom => ("custom", None, None),
            RunOrigin::Target { id, name } => ("target", Some(id.clone()), Some(name.clone())),
        };

        Self {
            source,
            target_id,
            target_name,
            target_path: request.path.display().to_string(),
            startup: request
                .startup
                .as_ref()
                .map(|startup| StartupConfiguration {
                    program: startup.command.program.clone(),
                    args: startup.command.args.clone(),
                    readiness: ReadinessConfiguration {
                        readiness_type: startup.readiness.readiness_type.clone(),
                        host: startup.readiness.host.clone(),
                        port: startup.readiness.port,
                        path: startup.readiness.path.clone(),
                        method: startup.readiness.method.clone(),
                        expected_status: startup.readiness.expected_status,
                        header_names: startup
                            .readiness
                            .headers
                            .as_ref()
                            .map(|headers| headers.keys().cloned().collect()),
                    },
                }),
            workload: request
                .workload
                .as_ref()
                .map(|workload| WorkloadConfiguration {
                    method: workload.method.clone(),
                    path: workload.path.clone(),
                    mode: if workload.requests.is_some() {
                        "requests"
                    } else {
                        "duration"
                    },
                    requests: workload.requests,
                    duration_seconds: workload.duration_seconds,
                    concurrency: workload.concurrency,
                    header_names: workload.headers.keys().cloned().collect(),
                }),
            workspaces: request
                .workspaces
                .iter()
                .map(WorkspaceConfiguration::from_request)
                .collect(),
        }
    }
}

impl WorkspaceConfiguration {
    pub fn from_request(workspace: &WorkspaceRequest) -> Self {
        let env = workspace.env.as_ref();

        let (env_source, env_file_path, env_variable_names) = match env {
            Some(env) => match (&env.file_path, &env.pasted) {
                (Some(path), None) => (Some("file".to_string()), Some(path.clone()), Vec::new()),
                (None, Some(text)) => (
                    Some("pasted".to_string()),
                    None,
                    // Validation already accepted the text, only the names are kept
                    env_file::parse(text)
                        .map(|variables| variables.into_keys().collect())
                        .unwrap_or_default(),
                ),
                _ => (None, None, Vec::new()),
            },
            None => (None, None, Vec::new()),
        };

        Self {
            name: workspace.name.clone(),
            directory: workspace.directory.clone(),
            env_source,
            env_file_path,
            env_variable_names,
            ready_log_line: workspace.ready_log_line.clone(),
            ready_timeout_seconds: workspace.ready_timeout_seconds,
            load_command: workspace.load.as_ref().map(|load| load.command.clone()),
            idle_seconds: workspace.idle_seconds,
            native: workspace
                .native
                .as_ref()
                .filter(|native| native.enabled)
                .map(|native| NativeConfiguration {
                    build_command: native.build_command.clone(),
                    command: native.command.clone(),
                }),
            docker: workspace
                .docker
                .as_ref()
                .filter(|docker| docker.enabled)
                .map(|docker| DockerConfiguration {
                    build_command: docker.build_command.clone(),
                    build_timeout_seconds: docker.build_timeout_seconds,
                    image: docker.image.clone(),
                    container_name: docker.container_name.clone(),
                    run_command: docker.run_command.clone(),
                }),
        }
    }
}

impl From<ReadinessConfiguration> for ReadinessSummary {
    fn from(readiness: ReadinessConfiguration) -> Self {
        Self {
            readiness_type: readiness.readiness_type,
            host: readiness.host,
            port: readiness.port,
            path: readiness.path,
            method: readiness.method,
            expected_status: readiness.expected_status,
            header_names: readiness.header_names,
        }
    }
}

impl From<StartupConfiguration> for StartupSummary {
    fn from(startup: StartupConfiguration) -> Self {
        Self {
            program: Some(startup.program),
            args: Some(startup.args),
            readiness: Some(startup.readiness.into()),
        }
    }
}
