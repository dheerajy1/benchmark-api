use serde::Deserialize;

use crate::persistence::history::CursorPosition;

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
}

// Validated list parameters
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListParams {
    pub limit: u32,
    pub before: Option<CursorPosition>,
    pub runtime: Option<String>,
    pub target_path: Option<String>,
}

impl ListQueryParams {
    pub fn validate(self) -> Result<ListParams, HistoryError> {
        Ok(ListParams {
            limit: parse_limit(self.limit.as_deref())?,
            before: self.before.as_deref().map(cursor::decode).transpose()?,
            runtime: non_empty("runtime", self.runtime)?,
            target_path: non_empty("target_path", self.target_path)?,
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
