use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

// Exact event names of the live-run contract
pub mod kind {
    pub const RUN_ACCEPTED: &str = "run.accepted";
    pub const STAGE_STARTED: &str = "stage.started";
    pub const STAGE_COMPLETED: &str = "stage.completed";
    pub const STAGE_SKIPPED: &str = "stage.skipped";
    pub const MEASUREMENT_AVAILABLE: &str = "measurement.available";
    pub const BASELINE_MEASURED: &str = "baseline.measured";
    pub const SAMPLE: &str = "sample";
    pub const RUN_COMPLETED: &str = "run.completed";
    pub const RUN_FAILED: &str = "run.failed";
    // Reserved for cancellation, never emitted in v1
    pub const RUN_CANCELLED: &str = "run.cancelled";
}

pub const CONTRACT_VERSION: u8 = 1;

// One ordered event of a run
#[derive(Debug, Clone, Serialize)]
pub struct RunEvent {
    pub v: u8,
    pub run_id: Uuid,
    pub seq: u64,
    pub t_ms: u64,
    pub at: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub data: Value,
}

// A terminal event ends the run, nothing can follow it
pub fn is_terminal(event_kind: &str) -> bool {
    event_kind == kind::RUN_COMPLETED
        || event_kind == kind::RUN_FAILED
        || event_kind == kind::RUN_CANCELLED
}

// Samples are the only events that may be evicted from a run's log
pub fn is_sample(event_kind: &str) -> bool {
    event_kind == kind::SAMPLE
}

// RFC 3339 with milliseconds in UTC, the format of every other timestamp of the API
pub fn format_timestamp(time: SystemTime) -> String {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));

    let total_seconds = duration.as_secs();
    let milliseconds = duration.subsec_millis();

    let days = total_seconds / 86_400;
    let seconds_of_day = total_seconds % 86_400;

    let (year, month, day) = civil_from_days(days as i64);

    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milliseconds:03}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };

    (year, month, day)
}
