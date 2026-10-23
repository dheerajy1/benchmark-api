use crate::errors::{api_error::ApiError, catalog::ErrorCode};

use super::slot::ActiveRun;
use crate::features::v1::benchmark::error::BenchmarkError;

// Why a submission was not accepted, shared by the legacy and the live path
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitError {
    // The request is invalid, no run was created and the slot was not touched
    Invalid(BenchmarkError),
    // Another benchmark holds the execution slot
    Busy(ActiveRun),
    // The run could not be set up, for example no cancel token could be created
    Internal(String),
}

impl From<SubmitError> for ApiError {
    fn from(error: SubmitError) -> Self {
        match error {
            SubmitError::Invalid(error) => ApiError::from(error),
            SubmitError::Busy(active) => ApiError::new(ErrorCode::Conflict, busy_detail(&active)),
            SubmitError::Internal(message) => {
                ApiError::new(ErrorCode::InternalServerError, message)
            }
        }
    }
}

// The error field names the active live run so a client can watch it, a legacy run has
// no id yet and gets a plain sentence
fn busy_detail(active: &ActiveRun) -> String {
    match active.run_id {
        Some(run_id) => run_id.to_string(),
        None => "A benchmark is already running.".to_string(),
    }
}
