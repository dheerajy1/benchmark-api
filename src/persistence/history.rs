use chrono::{DateTime, Utc};
use uuid::Uuid;

// Position of the last row of a page, the next page holds rows strictly older than it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPosition {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

// Filters and pagination of the history list, applied by the database
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryQuery {
    // Maximum number of rows to return
    pub limit: i64,
    // Only rows strictly older than this position, in (created_at, id) order
    pub after: Option<CursorPosition>,
    // Lookup code of the runtime, for example bun
    pub runtime_code: Option<String>,
    // Exact match on the stored target path
    pub target_path: Option<String>,
}

// One history row, canonical numbers straight from the normalized tables
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryRow {
    pub id: Uuid,
    pub target_path: String,
    pub runtime_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub build_time_ms: Option<i64>,
    pub startup_time_ms: Option<i64>,
    pub memory_peak_bytes: Option<i64>,
    pub cpu_average_percent: Option<f64>,
    pub requests_per_second: Option<f64>,
    pub p95_ms: Option<f64>,
}
