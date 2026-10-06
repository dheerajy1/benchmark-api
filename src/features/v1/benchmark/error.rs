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
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BenchmarkError::Validation(message)
            | BenchmarkError::InvalidPath(message)
            | BenchmarkError::Internal(message) => formatter.write_str(message),
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
        };

        ApiError::new(code, error.to_string())
    }
}
