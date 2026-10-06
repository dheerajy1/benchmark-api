use std::time::Duration;

use reqwest::header::HeaderMap;

use crate::features::v1::benchmark::{host::format_authority, request::WorkloadRequest};

use super::headers;

// Temporary internal default, not exposed as a request field yet
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

// Longest sustained workload a request may ask for
pub const MAX_DURATION_SECONDS: u64 = 300;

// What ends the workload
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadLimit {
    // Exactly this many requests
    Requests(u64),
    // Keep issuing requests until this much time has passed
    Duration(Duration),
}

// Validated, ready to execute workload
#[derive(Debug, Clone)]
pub struct WorkloadPlan {
    pub url: String,
    pub headers: HeaderMap,
    pub limit: WorkloadLimit,
    // Effective concurrency, capped by the request count in request-count mode
    pub concurrency: usize,
    pub request_timeout: Duration,
}

impl WorkloadPlan {
    pub fn new(
        request: &WorkloadRequest,
        host: &str,
        port: u16,
        request_timeout: Duration,
    ) -> Result<Self, String> {
        if !request.method.eq_ignore_ascii_case("GET") {
            return Err(format!(
                "Unsupported workload method: {} (only GET is supported)",
                request.method
            ));
        }

        if !request.path.starts_with('/') {
            return Err("Workload path must start with '/'".to_string());
        }

        let limit = resolve_limit(request)?;

        if request.concurrency == 0 {
            return Err("Workload concurrency must be greater than 0".to_string());
        }

        let url = format!("http://{}{}", format_authority(host, port), request.path);

        reqwest::Url::parse(&url).map_err(|error| format!("Invalid workload path: {error}"))?;

        let headers = headers::parse(&request.headers)?;

        let concurrency = match limit {
            WorkloadLimit::Requests(total) => request.concurrency.min(total),
            WorkloadLimit::Duration(_) => request.concurrency,
        };

        let concurrency = usize::try_from(concurrency)
            .map_err(|_| "Workload concurrency is too large".to_string())?;

        Ok(Self {
            url,
            headers,
            limit,
            concurrency,
            request_timeout,
        })
    }
}

// Exactly one of requests and duration_seconds must be present and valid
fn resolve_limit(request: &WorkloadRequest) -> Result<WorkloadLimit, String> {
    match (request.requests, request.duration_seconds) {
        (Some(_), Some(_)) => {
            Err("Workload requests and duration_seconds are mutually exclusive".to_string())
        }
        (None, None) => Err("Workload requires either requests or duration_seconds".to_string()),
        (Some(0), None) => Err("Workload requests must be greater than 0".to_string()),
        (Some(total), None) => Ok(WorkloadLimit::Requests(total)),
        (None, Some(0)) => Err("Workload duration_seconds must be greater than 0".to_string()),
        (None, Some(seconds)) if seconds > MAX_DURATION_SECONDS => Err(format!(
            "Workload duration_seconds must not exceed {MAX_DURATION_SECONDS}"
        )),
        (None, Some(seconds)) => Ok(WorkloadLimit::Duration(Duration::from_secs(seconds))),
    }
}
