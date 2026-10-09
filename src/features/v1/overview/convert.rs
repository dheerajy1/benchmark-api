use chrono::SecondsFormat;

use crate::persistence::{error::PersistenceError, history::TrendRow};

use super::{error::OverviewError, model::TrendPoint};

// Storage row to API point, nothing is calculated here: values pass through unchanged and
// a missing value stays None
pub fn to_trend_point(row: TrendRow) -> Result<TrendPoint, OverviewError> {
    Ok(TrendPoint {
        run_id: row.id,
        created_at: row.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        build_time_ms: optional_non_negative("build_time_ms", row.build_time_ms)?,
        startup_time_ms: optional_non_negative("startup_time_ms", row.startup_time_ms)?,
        memory_peak_bytes: optional_non_negative("memory_peak_bytes", row.memory_peak_bytes)?,
        cpu_average_percent: row.cpu_average_percent,
        requests_per_second: row.requests_per_second,
        p95_ms: row.p95_ms,
    })
}

// A count or measurement stored as BIGINT must not be negative, it never wraps silently
pub fn non_negative(field: &str, value: i64) -> Result<u64, OverviewError> {
    u64::try_from(value).map_err(|_| {
        OverviewError::Persistence(PersistenceError::InvalidResult(format!(
            "{field} is negative in the database: {value}"
        )))
    })
}

fn optional_non_negative(field: &str, value: Option<i64>) -> Result<Option<u64>, OverviewError> {
    value.map(|value| non_negative(field, value)).transpose()
}
