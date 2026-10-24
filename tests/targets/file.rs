use benchmark_api::features::v1::targets::catalog::{TargetCatalog, valid_target_id};
use tempfile::tempdir;

use super::support::{lookup, targets_json, write_file};

#[test]
fn a_valid_file_loads_with_literal_and_environment_headers() {
    let dir = tempdir().unwrap();
    let file = write_file(dir.path(), "targets.json", &targets_json(dir.path()));
    let lookup = lookup(&[("DEMO_SECRET", "value-from-env")]);

    let catalog = TargetCatalog::load(&file, &lookup).expect("the file loads");

    assert_eq!(catalog.len(), 2);
    let demo = catalog.get("demo").expect("demo exists");
    assert_eq!(demo.name, "Demo API");
    assert!(demo.startup.is_some());
    assert!(demo.workload.is_some());
}

#[test]
fn a_missing_environment_variable_is_named_without_a_value() {
    let dir = tempdir().unwrap();
    let file = write_file(dir.path(), "targets.json", &targets_json(dir.path()));
    let lookup = lookup(&[]);

    let error = TargetCatalog::load(&file, &lookup).expect_err("must fail");

    assert!(error.contains("DEMO_SECRET"), "{error}");
    assert!(error.contains("targets.demo"), "{error}");
}

#[test]
fn an_unknown_key_is_rejected_with_its_position_only() {
    let dir = tempdir().unwrap();
    let text = r#"{ "targets": { "demo": { "name": "Demo", "path": "/tmp", "unexpected": 1 } } }"#;
    let file = write_file(dir.path(), "targets.json", text);

    let error = TargetCatalog::load(&file, &lookup(&[])).expect_err("must fail");

    assert!(error.contains("line"), "{error}");
}

#[test]
fn an_invalid_target_id_is_rejected() {
    let dir = tempdir().unwrap();
    let text = r#"{ "targets": { "Bad_ID": { "name": "Demo", "path": "/tmp" } } }"#;
    let file = write_file(dir.path(), "targets.json", text);

    let error = TargetCatalog::load(&file, &lookup(&[])).expect_err("must fail");

    assert!(error.contains("Bad_ID"), "{error}");
}

#[test]
fn a_relative_path_is_rejected() {
    let dir = tempdir().unwrap();
    let text = r#"{ "targets": { "demo": { "name": "Demo", "path": "relative/dir" } } }"#;
    let file = write_file(dir.path(), "targets.json", text);

    let error = TargetCatalog::load(&file, &lookup(&[])).expect_err("must fail");

    assert!(error.contains("absolute"), "{error}");
}

#[test]
fn a_workload_needs_exactly_one_mode() {
    let dir = tempdir().unwrap();
    let text = format!(
        r#"{{ "targets": {{ "demo": {{ "name": "Demo", "path": "{}",
            "workload": {{ "method": "GET", "path": "/", "requests": 5,
                           "duration_seconds": 5, "concurrency": 1 }} }} }} }}"#,
        dir.path().display()
    );
    let file = write_file(dir.path(), "targets.json", &text);

    let error = TargetCatalog::load(&file, &lookup(&[])).expect_err("must fail");

    assert!(error.contains("exactly one"), "{error}");
}

#[test]
fn an_invalid_readiness_port_is_rejected_by_the_shared_validation() {
    let dir = tempdir().unwrap();
    let text = format!(
        r#"{{ "targets": {{ "demo": {{ "name": "Demo", "path": "{}",
            "startup": {{ "command": {{ "program": "bun" }},
                          "readiness": {{ "port": 0, "path": "/health", "expected_status": 200 }} }} }} }} }}"#,
        dir.path().display()
    );
    let file = write_file(dir.path(), "targets.json", &text);

    let error = TargetCatalog::load(&file, &lookup(&[])).expect_err("must fail");

    assert!(error.contains("targets.demo"), "{error}");
    assert!(error.contains("port"), "{error}");
}

#[test]
fn target_ids_follow_the_documented_pattern() {
    assert!(valid_target_id("demo"));
    assert!(valid_target_id("a-b-1"));
    assert!(!valid_target_id(""));
    assert!(!valid_target_id("-a"));
    assert!(!valid_target_id("A"));
    assert!(!valid_target_id(&"a".repeat(64)));
}
