use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use benchmark_api::{
    features::v1::benchmark::history::{
        cursor::{decode, encode},
        error::HistoryError,
    },
    persistence::history::CursorPosition,
};
use chrono::DateTime;
use uuid::Uuid;

fn position() -> CursorPosition {
    CursorPosition {
        // Microsecond precision, the precision PostgreSQL stores
        created_at: DateTime::from_timestamp(1_790_000_000, 123_456_000).unwrap(),
        id: Uuid::now_v7(),
    }
}

#[test]
fn a_cursor_round_trips_without_losing_precision() {
    let position = position();

    assert_eq!(decode(&encode(&position)).unwrap(), position);
}

#[test]
fn a_cursor_is_url_safe_base64_without_padding() {
    let cursor = encode(&position());

    assert!(!cursor.is_empty());
    assert!(!cursor.contains('='));
    assert!(
        cursor
            .chars()
            .all(|character| character.is_ascii_alphanumeric()
                || character == '-'
                || character == '_')
    );
}

#[test]
fn a_cursor_does_not_expose_its_content() {
    let position = position();
    let cursor = encode(&position);

    assert!(!cursor.contains(&position.id.to_string()));
    assert!(Uuid::parse_str(&cursor).is_err());
}

#[test]
fn garbage_is_rejected_as_a_validation_error() {
    let not_a_position = [
        "".to_string(),
        "not base64!!".to_string(),
        URL_SAFE_NO_PAD.encode("hello"),
        URL_SAFE_NO_PAD.encode("abc.def"),
        URL_SAFE_NO_PAD.encode("123.not-a-uuid"),
        URL_SAFE_NO_PAD.encode(format!("not-a-number.{}", Uuid::now_v7())),
        URL_SAFE_NO_PAD.encode([0xff, 0xfe, 0xfd]),
    ];

    for cursor in not_a_position {
        assert!(
            matches!(decode(&cursor), Err(HistoryError::Validation(_))),
            "cursor {cursor:?} must be rejected"
        );
    }
}
