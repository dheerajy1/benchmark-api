use std::collections::BTreeMap;

use crate::{
    errors::{api_error::ApiError, catalog::ErrorCode},
    features::v1::benchmark::{
        model::{BenchmarkRequest, BuildCommand},
        request::{ReadinessRequest, StartupRequest, WorkloadRequest},
        run_origin::RunOrigin,
        validation,
    },
};

use super::{
    catalog::{TargetEntry, valid_target_id},
    incoming::{IncomingRun, TargetRunBody, WorkloadOverride},
    policy::RunPolicy,
    secret::SecretValue,
};

// The request a run executes, and where it came from
pub struct ResolvedRun {
    pub request: BenchmarkRequest,
    pub origin: RunOrigin,
}

// Turns an incoming run into the request the benchmark runs. Raw requests are refused
// when custom runs are disabled; saved targets are always allowed.
pub fn resolve(policy: &RunPolicy, incoming: IncomingRun) -> Result<ResolvedRun, ApiError> {
    match incoming {
        IncomingRun::Raw(request) => {
            if !policy.custom_runs_allowed() {
                return Err(ApiError::new(
                    ErrorCode::Forbidden,
                    "Custom runs are disabled on this server, use a saved target.",
                ));
            }

            Ok(ResolvedRun {
                request: *request,
                origin: RunOrigin::Custom,
            })
        }
        IncomingRun::Target(body) => resolve_target(policy, body),
    }
}

fn resolve_target(policy: &RunPolicy, body: TargetRunBody) -> Result<ResolvedRun, ApiError> {
    if !valid_target_id(&body.target_id) {
        return Err(ApiError::new(
            ErrorCode::ValidationError,
            "target_id is not a valid saved-target id.",
        ));
    }

    let entry = policy
        .catalog()
        .get(&body.target_id)
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "No saved target has this id."))?;

    if body.workload.is_some() && entry.workload.is_none() {
        return Err(ApiError::new(
            ErrorCode::ValidationError,
            "This target has no workload, so workload overrides do not apply.",
        ));
    }

    let request = build_request(entry, body.workload.as_ref());

    validation::validate(&request)
        .map_err(|message| ApiError::new(ErrorCode::ValidationError, message))?;

    Ok(ResolvedRun {
        request,
        origin: RunOrigin::Target {
            id: entry.id.clone(),
            name: entry.name.clone(),
        },
    })
}

// The request for a target, with the workload overrides applied when given. Path, startup
// command, readiness and the workload's method, path and headers always come from the target.
pub fn build_request(
    entry: &TargetEntry,
    overrides: Option<&WorkloadOverride>,
) -> BenchmarkRequest {
    let startup = entry.startup.as_ref().map(|startup| StartupRequest {
        command: BuildCommand {
            program: startup.program.clone(),
            args: startup.args.clone(),
        },
        readiness: ReadinessRequest {
            readiness_type: "http".to_string(),
            host: startup.host.clone(),
            port: startup.port,
            path: startup.readiness_path.clone(),
            method: startup.method.clone(),
            expected_status: startup.expected_status,
            headers: startup.readiness_headers.as_deref().map(exposed),
        },
    });

    let workload = entry.workload.as_ref().map(|spec| {
        let (requests, duration_seconds, concurrency) = match overrides {
            None => (spec.requests, spec.duration_seconds, spec.concurrency),
            Some(overrides) => {
                let (requests, duration_seconds) =
                    match (overrides.requests, overrides.duration_seconds) {
                        (Some(requests), _) => (Some(requests), None),
                        (None, Some(duration)) => (None, Some(duration)),
                        (None, None) => (spec.requests, spec.duration_seconds),
                    };

                (
                    requests,
                    duration_seconds,
                    overrides.concurrency.unwrap_or(spec.concurrency),
                )
            }
        };

        WorkloadRequest {
            method: spec.method.clone(),
            path: spec.path.clone(),
            headers: exposed(&spec.headers),
            requests,
            duration_seconds,
            concurrency,
        }
    });

    BenchmarkRequest {
        path: entry.path.clone(),
        startup,
        workload,
    }
}

// Secret values leave the wrapper only here, for the outgoing request
fn exposed(headers: &[(String, SecretValue)]) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose().to_string()))
        .collect()
}
