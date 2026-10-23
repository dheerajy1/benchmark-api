use std::fmt;

use crate::errors::{api_error::ApiError, catalog::ErrorCode};

// Failures of a benchmark run, mapped to the global API error afterwards
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchmarkError {
    // Request configuration rejected by validation
    Validation(String),
    // Target path missing or not a directory
    InvalidPath(String),
    // Failure while running the benchmark itself
    Internal(String),
    // The run was stopped on request, only a live run can be cancelled
    Cancelled,
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BenchmarkError::Validation(message)
            | BenchmarkError::InvalidPath(message)
            | BenchmarkError::Internal(message) => formatter.write_str(message),
            BenchmarkError::Cancelled => formatter.write_str("The benchmark was cancelled."),
        }
    }
}

impl std::error::Error for BenchmarkError {}

// Plain string errors from the metric modules are internal failures
impl From<String> for BenchmarkError {
    fn from(message: String) -> Self {
        BenchmarkError::Internal(message)
    }
}

impl From<BenchmarkError> for ApiError {
    fn from(error: BenchmarkError) -> Self {
        let code = match &error {
            BenchmarkError::Validation(_) => ErrorCode::ValidationError,
            BenchmarkError::InvalidPath(_) => ErrorCode::BadRequest,
            BenchmarkError::Internal(_) => ErrorCode::InternalServerError,
            BenchmarkError::Cancelled => ErrorCode::Conflict,
        };

        ApiError::new(code, error.to_string())
    }
}

impl From<super::stop::Stop> for BenchmarkError {
    fn from(stop: super::stop::Stop) -> Self {
        match stop {
            super::stop::Stop::Cancelled => BenchmarkError::Cancelled,
            super::stop::Stop::Failed(message) => BenchmarkError::Internal(message),
        }
    }
}
