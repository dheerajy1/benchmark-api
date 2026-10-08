use serde::Deserialize;
use uuid::Uuid;

use super::error::ComparisonError;

// Raw query string, runIds is one comma separated value
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonQueryParams {
    pub run_ids: Option<String>,
}

// The two validated runs, the order is the order of the request and is never changed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunIds {
    pub baseline: Uuid,
    pub comparison: Uuid,
}

impl ComparisonQueryParams {
    pub fn validate(self) -> Result<RunIds, ComparisonError> {
        let raw = self.run_ids.unwrap_or_default();

        if raw.is_empty() {
            return Err(ComparisonError::Validation(
                "runIds is required: exactly two run ids separated by a comma, \
                 baseline first."
                    .to_string(),
            ));
        }

        let segments: Vec<&str> = raw.split(',').collect();

        if segments.len() != 2 {
            return Err(ComparisonError::Validation(format!(
                "runIds must contain exactly two run ids, received {}.",
                segments.len()
            )));
        }

        // Strict: spaces and empty segments are not valid UUIDs and are rejected as such
        let baseline = parse_run_id("baseline", segments[0])?;
        let comparison = parse_run_id("comparison", segments[1])?;

        // Compared after parsing, so letter case cannot hide a duplicate
        if baseline == comparison {
            return Err(ComparisonError::Validation(
                "runIds must be two different runs, received the same run id twice.".to_string(),
            ));
        }

        Ok(RunIds {
            baseline,
            comparison,
        })
    }
}

fn parse_run_id(role: &str, value: &str) -> Result<Uuid, ComparisonError> {
    Uuid::parse_str(value).map_err(|_| {
        ComparisonError::Validation(format!("The {role} run id '{value}' is not a valid UUID."))
    })
}
