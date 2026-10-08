use std::fmt;

use crate::{
    errors::{api_error::ApiError, catalog::ErrorCode},
    persistence::error::PersistenceError,
};

// Failures of the comparison endpoint, mapped to the global API error afterwards
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComparisonError {
    // Query parameter rejected by validation
    Validation(String),
    // One or both requested runs are not stored
    NotFound(String),
    // The database failed or holds data the API cannot represent
    Persistence(PersistenceError),
}

impl fmt::Display for ComparisonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComparisonError::Validation(message) | ComparisonError::NotFound(message) => {
                formatter.write_str(message)
            }
            ComparisonError::Persistence(error) => {
                write!(formatter, "Failed to read the benchmark runs: {error}")
            }
        }
    }
}

impl std::error::Error for ComparisonError {}

impl From<PersistenceError> for ComparisonError {
    fn from(error: PersistenceError) -> Self {
        ComparisonError::Persistence(error)
    }
}

impl From<ComparisonError> for ApiError {
    fn from(error: ComparisonError) -> Self {
        let code = match &error {
            ComparisonError::Validation(_) => ErrorCode::ValidationError,
            ComparisonError::NotFound(_) => ErrorCode::NotFound,
            ComparisonError::Persistence(_) => ErrorCode::InternalServerError,
        };

        ApiError::new(code, error.to_string())
    }
}
