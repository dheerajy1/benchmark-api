use benchmark_api::persistence::{
    error::PersistenceError,
    postgres::mapping::{map_result, parse_timestamp, run_created_at, to_i64},
};
use chrono::SecondsFormat;
use uuid::{NoContext, Timestamp, Uuid};

use super::sample;

#[test]
fn parses_an_rfc3339_timestamp() {
    let parsed = parse_timestamp("field", Some("2026-10-04T12:00:01.842Z"))
        .unwrap()
        .unwrap();

    assert_eq!(
        parsed.to_rfc3339_opts(SecondsFormat::Millis, true),
        "2026-10-04T12:00:01.842Z"
    );
}

#[test]
fn an_absent_timestamp_stays_absent() {
    assert_eq!(parse_timestamp("field", None), Ok(None));
}

#[test]
fn an_invalid_timestamp_is_an_invalid_result() {
    let error = parse_timestamp("started_at", Some("yesterday")).unwrap_err();

    assert!(matches!(error, PersistenceError::InvalidResult(_)));
    assert!(error.to_string().contains("started_at"));
}

#[test]
fn converts_u64_values_that_fit_a_bigint() {
    assert_eq!(to_i64("field", 0), Ok(0));
    assert_eq!(to_i64("field", i64::MAX as u64), Ok(i64::MAX));
}

#[test]
fn rejects_u64_values_that_do_not_fit_a_bigint() {
    let error = to_i64("field", i64::MAX as u64 + 1).unwrap_err();

    assert!(matches!(error, PersistenceError::InvalidResult(_)));
}

#[test]
fn created_at_is_the_timestamp_inside_the_uuid_v7() {
    let run_id = Uuid::new_v7(Timestamp::from_unix(NoContext, 1_791_000_000, 123_000_000));

    let created_at = run_created_at(run_id).unwrap();

    assert_eq!(created_at.timestamp(), 1_791_000_000);
    assert_eq!(created_at.timestamp_subsec_millis(), 123);
}

#[test]
fn created_at_rejects_an_id_without_a_timestamp() {
    assert!(matches!(
        run_created_at(Uuid::nil()),
        Err(PersistenceError::InvalidResult(_))
    ));
}

#[test]
fn maps_a_complete_result() {
    let result = sample::result();

    let rows = map_result(&result).unwrap();

    assert_eq!(rows.run.id, result.run_id);
    assert_eq!(rows.run.target_path, "/absolute/path/to/project");
    assert_eq!(rows.run.runtime_code.as_deref(), Some("bun"));

    let artifacts: Vec<(&str, i64, i64)> = rows
        .artifact_sizes
        .iter()
        .map(|row| (row.artifact_type_code, row.bytes, row.file_count))
        .collect();

    assert_eq!(
        artifacts,
        vec![("source", 100, 1), ("build", 200, 2), ("excluded", 300, 3)]
    );

    assert_eq!(rows.build.status_code, "success");
    assert_eq!(rows.build.duration_ms, Some(1842));
    assert_eq!(rows.build.program.as_deref(), Some("bun"));
    assert_eq!(
        rows.build.args,
        Some(vec!["run".to_string(), "build".to_string()])
    );
    assert_eq!(rows.build.exit_code, Some(0));

    assert_eq!(rows.startup.status_code, "success");
    assert_eq!(rows.startup.readiness_port, Some(5001));
    assert_eq!(rows.startup.readiness_expected_status, Some(200));
    assert!(rows.startup.ready_at.is_some());

    assert_eq!(rows.memory.unwrap().peak_rss_bytes, 73_400_320);
    assert_eq!(rows.cpu.unwrap().average_percent, 12.4);

    let http = rows.http_latency.unwrap();

    assert_eq!(http.status_code, "completed");
    assert_eq!(http.total_requests, 1000);
    assert_eq!(http.p99_ms, Some(15.2));
}

#[test]
fn maps_a_skipped_result_without_optional_metrics() {
    let rows = map_result(&sample::skipped_result()).unwrap();

    assert_eq!(rows.run.runtime_code, None);

    assert_eq!(rows.build.status_code, "no_build_command");
    assert_eq!(rows.build.program, None);
    assert_eq!(rows.build.started_at, None);

    assert_eq!(rows.startup.status_code, "no_start_command");
    assert_eq!(rows.startup.readiness_type, None);

    assert!(rows.memory.is_none());
    assert!(rows.cpu.is_none());
    assert!(rows.http_latency.is_none());
}

#[test]
fn a_successful_startup_without_ready_at_is_rejected() {
    let mut result = sample::result();

    result.metrics.startup_time.ready_at = None;

    assert!(matches!(
        map_result(&result),
        Err(PersistenceError::InvalidResult(_))
    ));
}

#[test]
fn a_value_beyond_bigint_is_rejected() {
    let mut result = sample::result();

    result.metrics.artifact_size.source.bytes = u64::MAX;

    assert!(matches!(
        map_result(&result),
        Err(PersistenceError::InvalidResult(_))
    ));
}

#[test]
fn a_run_id_without_a_timestamp_is_rejected() {
    let mut result = sample::result();

    result.run_id = Uuid::nil();

    assert!(matches!(
        map_result(&result),
        Err(PersistenceError::InvalidResult(_))
    ));
}
