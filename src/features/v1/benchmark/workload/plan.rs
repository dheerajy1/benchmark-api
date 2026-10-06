use std::time::Duration;

use reqwest::header::HeaderMap;

use crate::features::v1::benchmark::{host::format_authority, request::WorkloadRequest};

use super::headers;

// Temporary internal default, not exposed as a request field yet
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

// Validated, ready to execute workload
#[derive(Debug, Clone)]
pub struct WorkloadPlan {
    pub url: String,
    pub headers: HeaderMap,
    pub requests: u64,
    // Effective concurrency, never above the request count
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

        if request.requests == 0 {
            return Err("Workload requests must be greater than 0".to_string());
        }

        if request.concurrency == 0 {
            return Err("Workload concurrency must be greater than 0".to_string());
        }

        let url = format!("http://{}{}", format_authority(host, port), request.path);

        reqwest::Url::parse(&url).map_err(|error| format!("Invalid workload path: {error}"))?;

        let headers = headers::parse(&request.headers)?;

        let concurrency = usize::try_from(request.concurrency.min(request.requests))
            .map_err(|_| "Workload concurrency is too large".to_string())?;

        Ok(Self {
            url,
            headers,
            requests: request.requests,
            concurrency,
            request_timeout,
        })
    }
}
