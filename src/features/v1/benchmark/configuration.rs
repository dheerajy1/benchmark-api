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
