use serde::Serialize;

use crate::features::v1::benchmark::configuration::WorkspaceConfiguration;

use super::{
    catalog::{StartupSpec, TargetEntry, WorkloadSpec},
    policy::RunPolicy,
    secret::SecretValue,
};

// GET /targets: what a client may read. Header names only, never a value.
#[derive(Debug, Serialize)]
pub struct TargetsView {
    pub custom_runs_allowed: bool,
    pub targets: Vec<TargetView>,
}

#[derive(Debug, Serialize)]
pub struct TargetView {
    pub id: String,
    pub name: String,
    pub path: String,
    pub startup: Option<StartupView>,
    pub workload: Option<WorkloadView>,
    // Variable names only, never a pasted value
    pub workspaces: Vec<WorkspaceConfiguration>,
}

#[derive(Debug, Serialize)]
pub struct StartupView {
    pub command: CommandView,
    pub readiness: ReadinessView,
}

#[derive(Debug, Serialize)]
pub struct CommandView {
    pub program: String,
    pub args: Vec<String>,
}

// header_names is null when the server's health credentials are used
#[derive(Debug, Serialize)]
pub struct ReadinessView {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub method: String,
    pub expected_status: u16,
    pub header_names: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct WorkloadView {
    pub method: String,
    pub path: String,
    pub mode: &'static str,
    pub requests: Option<u64>,
    pub duration_seconds: Option<u64>,
    pub concurrency: u64,
    pub header_names: Vec<String>,
}

impl TargetsView {
    pub fn from_policy(policy: &RunPolicy) -> Self {
        Self {
            custom_runs_allowed: policy.custom_runs_allowed(),
            targets: policy
                .catalog()
                .entries()
                .map(TargetView::from_entry)
                .collect(),
        }
    }
}

impl TargetView {
    fn from_entry(entry: &TargetEntry) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            path: entry.path.display().to_string(),
            startup: entry.startup.as_ref().map(StartupView::from_spec),
            workload: entry.workload.as_ref().map(WorkloadView::from_spec),
            workspaces: entry
                .workspaces
                .iter()
                .map(WorkspaceConfiguration::from_request)
                .collect(),
        }
    }
}

impl StartupView {
    fn from_spec(spec: &StartupSpec) -> Self {
        Self {
            command: CommandView {
                program: spec.program.clone(),
                args: spec.args.clone(),
            },
            readiness: ReadinessView {
                host: spec.host.clone(),
                port: spec.port,
                path: spec.readiness_path.clone(),
                method: spec.method.clone(),
                expected_status: spec.expected_status,
                header_names: spec.readiness_headers.as_deref().map(header_names),
            },
        }
    }
}

impl WorkloadView {
    fn from_spec(spec: &WorkloadSpec) -> Self {
        Self {
            method: spec.method.clone(),
            path: spec.path.clone(),
            mode: if spec.requests.is_some() {
                "requests"
            } else {
                "duration"
            },
            requests: spec.requests,
            duration_seconds: spec.duration_seconds,
            concurrency: spec.concurrency,
            header_names: header_names(&spec.headers),
        }
    }
}

fn header_names(headers: &[(String, SecretValue)]) -> Vec<String> {
    headers.iter().map(|(name, _)| name.clone()).collect()
}
