use serde::Serialize;

use super::{model::BenchmarkRequest, run_origin::RunOrigin};

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
