use serde::Deserialize;

use crate::persistence::{
    history::CursorPosition,
    outcome::{STATUS_CANCELLED, STATUS_DONE, STATUS_FAILED, StoredStatus},
    path::trim_trailing_slashes,
};

use super::{cursor, error::HistoryError};

pub const DEFAULT_LIMIT: u32 = 20;
pub const MAX_LIMIT: u32 = 100;

// Raw query string values, every value is validated by hand for a precise error
#[derive(Debug, Default, Deserialize)]
pub struct ListQueryParams {
    pub limit: Option<String>,
    pub before: Option<String>,
    pub runtime: Option<String>,
    pub target_path: Option<String>,
    pub status: Option<String>,
    pub search: Option<String>,
}

// Validated list parameters
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListParams {
    pub limit: u32,
    pub before: Option<CursorPosition>,
    pub runtime: Option<String>,
    pub target_path: Option<String>,
    pub status: Option<StoredStatus>,
    pub search: Option<String>,
}

impl ListQueryParams {
    pub fn validate(self) -> Result<ListParams, HistoryError> {
        Ok(ListParams {
            limit: parse_limit(self.limit.as_deref())?,
            before: self.before.as_deref().map(cursor::decode).transpose()?,
            runtime: non_empty("runtime", self.runtime)?,
            // A pinned target always matches its runs, however the path is spelled
            target_path: non_empty("target_path", self.target_path)?
                .map(|path| trim_trailing_slashes(&path).to_string()),
            status: parse_status(self.status.as_deref())?,
            search: parse_search(self.search)?,
        })
    }
}

fn parse_limit(value: Option<&str>) -> Result<u32, HistoryError> {
    let Some(value) = value else {
        return Ok(DEFAULT_LIMIT);
    };

    let invalid = || {
        HistoryError::Validation(format!(
            "limit must be an integer between 1 and {MAX_LIMIT}."
        ))
    };

    let limit: i64 = value.parse().map_err(|_| invalid())?;

    if limit < 1 || limit > i64::from(MAX_LIMIT) {
        return Err(invalid());
    }

    u32::try_from(limit).map_err(|_| invalid())
}

fn non_empty(name: &str, value: Option<String>) -> Result<Option<String>, HistoryError> {
    match value {
        Some(value) if value.is_empty() => Err(HistoryError::Validation(format!(
            "{name} must not be empty."
        ))),
        other => Ok(other),
    }
}

fn parse_status(value: Option<&str>) -> Result<Option<StoredStatus>, HistoryError> {
    value
        .map(|value| {
            StoredStatus::from_code(value).ok_or_else(|| {
                HistoryError::Validation(format!(
                    "status must be one of {STATUS_DONE}, {STATUS_FAILED}, {STATUS_CANCELLED}."
                ))
            })
        })
        .transpose()
}

// Spaces around the text are not part of the search
fn parse_search(value: Option<String>) -> Result<Option<String>, HistoryError> {
    match value.map(|value| value.trim().to_string()) {
        Some(value) if value.is_empty() => Err(HistoryError::Validation(
            "search must not be empty.".to_string(),
        )),
        other => Ok(other),
    }
}
