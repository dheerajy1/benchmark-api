use std::fmt;

use crate::{
    errors::{api_error::ApiError, catalog::ErrorCode},
    features::v1::benchmark::history::error::HistoryError,
    persistence::error::PersistenceError,
};

// Failures of the overview endpoint, mapped to the global API error afterwards.
// The endpoint has no parameters, so the only failure is storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverviewError {
    // The database failed or holds data the API cannot represent
    Persistence(PersistenceError),
}

impl fmt::Display for OverviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OverviewError::Persistence(error) => {
                write!(formatter, "Failed to read the benchmark overview: {error}")
            }
        }
    }
}

impl std::error::Error for OverviewError {}

impl From<PersistenceError> for OverviewError {
    fn from(error: PersistenceError) -> Self {
        OverviewError::Persistence(error)
    }
}

// The latest run is read through the history service. Its list is called with fixed valid
// parameters, so a validation or not found failure cannot happen and is reported as an
// unrepresentable result instead of being hidden.
impl From<HistoryError> for OverviewError {
    fn from(error: HistoryError) -> Self {
        match error {
            HistoryError::Persistence(error) => OverviewError::Persistence(error),
            other => OverviewError::Persistence(PersistenceError::InvalidResult(other.to_string())),
        }
    }
}

impl From<OverviewError> for ApiError {
    fn from(error: OverviewError) -> Self {
        let code = match &error {
            OverviewError::Persistence(_) => ErrorCode::InternalServerError,
        };

        ApiError::new(code, error.to_string())
    }
}
