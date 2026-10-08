use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::DateTime;
use uuid::Uuid;

use crate::persistence::history::CursorPosition;

use super::error::HistoryError;

// The cursor is Base64 text, clients must treat it as opaque and only send it back.
// Inside it holds the (created_at, id) position as "<unix microseconds>.<uuid>".
pub fn encode(position: &CursorPosition) -> String {
    let text = format!("{}.{}", position.created_at.timestamp_micros(), position.id);

    URL_SAFE_NO_PAD.encode(text)
}

pub fn decode(cursor: &str) -> Result<CursorPosition, HistoryError> {
    let invalid = || {
        HistoryError::Validation(
            "before must be a cursor returned by a previous response.".to_string(),
        )
    };

    let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
    let text = String::from_utf8(bytes).map_err(|_| invalid())?;

    let (micros, id) = text.split_once('.').ok_or_else(invalid)?;

    let micros: i64 = micros.parse().map_err(|_| invalid())?;
    let id = Uuid::parse_str(id).map_err(|_| invalid())?;

    let created_at = DateTime::from_timestamp(
        micros.div_euclid(1_000_000),
        (micros.rem_euclid(1_000_000) as u32) * 1_000,
    )
    .ok_or_else(invalid)?;

    Ok(CursorPosition { created_at, id })
}
