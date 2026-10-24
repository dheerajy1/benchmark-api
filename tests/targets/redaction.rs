use benchmark_api::{
    features::v1::{
        benchmark::{
            model::BenchmarkRequest,
            request::{ReadinessRequest, WorkloadRequest},
        },
        targets::{catalog::TargetCatalog, resolve::build_request, secret::SecretValue},
    },
    schemas::env::EnvSchema,
};
use serde_json::json;
use tempfile::tempdir;

use super::support::{lookup, targets_json, write_file};

#[test]
fn a_secret_value_prints_as_redacted() {
    let value = SecretValue::new("SECRET-LITERAL-A".to_string());

    assert_eq!(format!("{value:?}"), "<redacted>");
    assert_eq!(value.expose(), "SECRET-LITERAL-A");
}

#[test]
fn the_environment_schema_hides_the_health_secret() {
    let schema = EnvSchema {
        app_port: 5001,
        app_health_client_id: "client".to_string(),
        app_health_client_secret: "SECRET-LITERAL-B".to_string(),
    };

    let printed = format!("{schema:?}");

    assert!(!printed.contains("SECRET-LITERAL-B"), "{printed}");
    assert!(printed.contains("<redacted>"), "{printed}");
}

#[test]
fn request_types_show_header_names_but_no_values() {
    let readiness: ReadinessRequest = serde_json::from_value(json!({
        "type": "http", "host": "127.0.0.1", "port": 5000, "path": "/health",
        "method": "GET", "expected_status": 200,
        "headers": { "X-Client-Secret": "SECRET-LITERAL-C" }
    }))
    .unwrap();

    let workload: WorkloadRequest = serde_json::from_value(json!({
        "method": "GET", "path": "/", "requests": 5, "concurrency": 1,
        "headers": { "X-Other": "SECRET-LITERAL-D" }
    }))
    .unwrap();

    let printed = format!("{readiness:?} {workload:?}");

    assert!(printed.contains("X-Client-Secret"), "{printed}");
    assert!(printed.contains("X-Other"), "{printed}");
    assert!(!printed.contains("SECRET-LITERAL-C"), "{printed}");
    assert!(!printed.contains("SECRET-LITERAL-D"), "{printed}");
}

#[test]
fn a_resolved_target_keeps_its_secret_for_the_request_only() {
    let dir = tempdir().unwrap();
    let file = write_file(dir.path(), "targets.json", &targets_json(dir.path()));
    let lookup = lookup(&[("DEMO_SECRET", "ENV-VALUE-XYZ")]);
    let catalog = TargetCatalog::load(&file, &lookup).unwrap();

    // The catalog and anything built from it print without the value
    assert!(!format!("{catalog:?}").contains("ENV-VALUE-XYZ"));

    let request: BenchmarkRequest = build_request(catalog.get("demo").unwrap(), None);

    assert!(!format!("{request:?}").contains("ENV-VALUE-XYZ"));

    // The outgoing request still carries the real value
    let sent = request
        .workload
        .as_ref()
        .unwrap()
        .headers
        .get("X-Client-Secret")
        .unwrap();

    assert_eq!(sent, "ENV-VALUE-XYZ");
}
