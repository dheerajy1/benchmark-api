use axum::{
    body::{Body, Bytes},
    extract::{FromRequest, Request},
    http::HeaderMap,
};
use serde_json::{Map, Value};

use crate::{
    app_state::AppState,
    errors::{api_error::ApiError, catalog::ErrorCode, json::ApiJson},
    features::v1::benchmark::model::BenchmarkRequest,
};

// One run request as it arrived: a raw request, or a reference to a saved target
pub enum IncomingRun {
    Raw(BenchmarkRequest),
    Target(TargetRunBody),
}

// A saved-target run: the target and the workload settings it may override
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRunBody {
    pub target_id: String,
    pub workload: Option<WorkloadOverride>,
}

// The only values a saved-target run may change, each optional
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadOverride {
    pub requests: Option<u64>,
    pub duration_seconds: Option<u64>,
    pub concurrency: Option<u64>,
}

impl FromRequest<AppState> for IncomingRun {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        let headers = request.headers().clone();

        let body = Bytes::from_request(request, state)
            .await
            .map_err(|rejection| ApiError::new(ErrorCode::BadRequest, rejection.body_text()))?;

        // Content type and JSON syntax are checked by the same extractor as before
        let ApiJson(value) =
            ApiJson::<Value>::from_request(rebuild(&headers, body.clone()), state).await?;

        if is_target_body(&value) {
            return TargetRunBody::from_value(value).map(IncomingRun::Target);
        }

        // Raw requests go through the original extractor, so their errors stay the same
        let ApiJson(request) =
            ApiJson::<BenchmarkRequest>::from_request(rebuild(&headers, body), state).await?;

        Ok(IncomingRun::Raw(request))
    }
}

// The same headers and the buffered body, so the JSON extractor can read it again
fn rebuild(headers: &HeaderMap, body: Bytes) -> Request {
    let mut request = Request::new(Body::from(body));
    *request.headers_mut() = headers.clone();
    request
}

// A body that names a target is handled here, anything else is a raw request
fn is_target_body(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.contains_key("target_id"))
}

impl TargetRunBody {
    fn from_value(value: Value) -> Result<Self, ApiError> {
        let Value::Object(mut object) = value else {
            return Err(invalid("The request body must be a JSON object."));
        };

        // Only the names of keys are reported, never their values
        for key in object.keys() {
            if !matches!(key.as_str(), "target_id" | "workload") {
                return Err(invalid(format!(
                    "A saved-target run accepts only target_id and workload, not {key}."
                )));
            }
        }

        let target_id = match object.remove("target_id") {
            Some(Value::String(id)) => id,
            _ => return Err(invalid("target_id must be a string.")),
        };

        let workload = match object.remove("workload") {
            None | Some(Value::Null) => None,
            Some(value) => Some(WorkloadOverride::from_value(value)?),
        };

        Ok(Self {
            target_id,
            workload,
        })
    }
}

impl WorkloadOverride {
    fn from_value(value: Value) -> Result<Self, ApiError> {
        let Value::Object(object) = value else {
            return Err(invalid("workload must be a JSON object."));
        };

        for key in object.keys() {
            if !matches!(
                key.as_str(),
                "requests" | "duration_seconds" | "concurrency"
            ) {
                return Err(invalid(format!(
                    "A workload override accepts only requests, duration_seconds and concurrency, not {key}."
                )));
            }
        }

        let requests = whole_number(&object, "requests")?;
        let duration_seconds = whole_number(&object, "duration_seconds")?;
        let concurrency = whole_number(&object, "concurrency")?;

        if requests.is_some() && duration_seconds.is_some() {
            return Err(invalid(
                "Use either workload.requests or workload.duration_seconds, not both.",
            ));
        }

        if requests.is_none() && duration_seconds.is_none() && concurrency.is_none() {
            return Err(invalid(
                "A workload override needs requests, duration_seconds or concurrency.",
            ));
        }

        Ok(Self {
            requests,
            duration_seconds,
            concurrency,
        })
    }
}

fn whole_number(object: &Map<String, Value>, field: &str) -> Result<Option<u64>, ApiError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| invalid(format!("workload.{field} must be a positive whole number."))),
    }
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::ValidationError, message)
}
