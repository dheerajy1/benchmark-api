use std::fmt;

use crate::{
    errors::{api_error::ApiError, catalog::ErrorCode},
    persistence::error::PersistenceError,
};

// Failures of the history endpoints, mapped to the global API error afterwards
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    // Query parameter rejected by validation
    Validation(String),
    // A syntactically valid run id that is not stored
    NotFound(String),
    // The database failed or holds data the API cannot represent
    Persistence(PersistenceError),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::Validation(message) | HistoryError::NotFound(message) => {
                formatter.write_str(message)
            }
            HistoryError::Persistence(error) => {
                write!(formatter, "Failed to read benchmark history: {error}")
            }
        }
    }
}

impl std::error::Error for HistoryError {}

impl From<PersistenceError> for HistoryError {
    fn from(error: PersistenceError) -> Self {
        HistoryError::Persistence(error)
    }
}

impl From<HistoryError> for ApiError {
    fn from(error: HistoryError) -> Self {
        let code = match &error {
            HistoryError::Validation(_) => ErrorCode::ValidationError,
            HistoryError::NotFound(_) => ErrorCode::NotFound,
            HistoryError::Persistence(_) => ErrorCode::InternalServerError,
        };

        ApiError::new(code, error.to_string())
    }
}
