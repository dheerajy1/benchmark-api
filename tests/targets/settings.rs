use benchmark_api::schemas::runs::RunsSchema;
use serde_json::json;
use validator::Validate;

#[test]
fn custom_runs_default_to_disabled_and_no_targets_file() {
    let schema: RunsSchema = serde_json::from_value(json!({})).unwrap();

    assert!(!schema.allow_custom_runs);
    assert!(schema.targets_file.is_none());
}

#[test]
fn a_relative_targets_file_is_rejected_by_the_settings_check() {
    let schema = RunsSchema {
        allow_custom_runs: true,
        targets_file: Some("relative/targets.json".to_string()),
    };

    assert!(schema.validate().is_ok());
    assert!(schema.check_targets_file().is_err());
}

#[test]
fn an_absolute_targets_file_is_accepted() {
    let schema = RunsSchema {
        allow_custom_runs: false,
        targets_file: Some("/srv/benchmark/targets.json".to_string()),
    };

    assert!(schema.validate().is_ok());
    assert!(schema.check_targets_file().is_ok());
}

#[test]
fn an_overlong_targets_file_fails_validation() {
    let schema = RunsSchema {
        allow_custom_runs: false,
        targets_file: Some(format!("/{}", "a".repeat(5000))),
    };

    assert!(schema.validate().is_err());
}
