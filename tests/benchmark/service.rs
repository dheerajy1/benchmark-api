use std::fs::{self, File};

use benchmark_api::config::env::Env;
use benchmark_api::features::v1::benchmark::{
    error::BenchmarkError, model::BenchmarkRequest, service,
};
use tempfile::TempDir;
use uuid::Uuid;

fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

#[test]
fn runs_benchmark_and_assembles_result() {
    let temp_dir = TempDir::new().unwrap();

    fs::create_dir_all(temp_dir.path().join("target")).unwrap();

    File::create(temp_dir.path().join("source.txt"))
        .unwrap()
        .set_len(100)
        .unwrap();

    File::create(temp_dir.path().join("target").join("artifact"))
        .unwrap()
        .set_len(200)
        .unwrap();

    fs::create_dir_all(temp_dir.path().join("node_modules")).unwrap();

    File::create(temp_dir.path().join("node_modules").join("dependency"))
        .unwrap()
        .set_len(300)
        .unwrap();

    let request = BenchmarkRequest {
        path: temp_dir.path().to_path_buf(),
        startup: None,
        workload: None,
        workspaces: Vec::new(),
    };

    let result = service::run(&request, &test_env()).unwrap();

    assert_eq!(result.run_id.get_version_num(), 7);
    assert_eq!(result.target.path, temp_dir.path().display().to_string());

    assert!(result.discovery.runtime.is_none());
    assert!(result.discovery.build.is_none());

    assert!(result.metrics.memory.is_none());
    assert!(result.metrics.http_latency.is_none());

    assert_eq!(result.metrics.artifact_size.source.bytes, 100);
    assert_eq!(result.metrics.artifact_size.source.size, "100 B");
    assert_eq!(result.metrics.artifact_size.source.file_count, 1);

    assert_eq!(result.metrics.artifact_size.build.bytes, 200);
    assert_eq!(result.metrics.artifact_size.build.size, "200 B");
    assert_eq!(result.metrics.artifact_size.build.file_count, 1);

    assert_eq!(result.metrics.artifact_size.excluded.bytes, 300);
    assert_eq!(result.metrics.artifact_size.excluded.size, "300 B");
    assert_eq!(result.metrics.artifact_size.excluded.file_count, 1);

    assert!(!Uuid::nil().eq(&result.run_id));
}

#[test]
fn rejects_invalid_workload_before_running_anything() {
    let temp_dir = TempDir::new().unwrap();

    let request: BenchmarkRequest = serde_json::from_value(serde_json::json!({
        "path": temp_dir.path().display().to_string(),
        "workload": {
            "method": "GET",
            "path": "/health",
            "requests": 0,
            "concurrency": 1
        }
    }))
    .unwrap();

    let error = service::run(&request, &test_env()).err().unwrap();

    assert!(matches!(error, BenchmarkError::Validation(_)));
    assert!(error.to_string().contains("requests"));
}

#[test]
fn missing_path_is_reported_as_an_invalid_path() {
    let request: BenchmarkRequest = serde_json::from_value(serde_json::json!({
        "path": "/definitely/not/a/real/path"
    }))
    .unwrap();

    let error = service::run(&request, &test_env()).err().unwrap();

    assert!(matches!(error, BenchmarkError::InvalidPath(_)));
    assert!(error.to_string().contains("Path does not exist"));
}

#[test]
fn file_path_is_reported_as_an_invalid_path() {
    let temp_dir = TempDir::new().unwrap();
    let file = temp_dir.path().join("file.txt");

    std::fs::write(&file, "x").unwrap();

    let request: BenchmarkRequest = serde_json::from_value(serde_json::json!({
        "path": file.display().to_string()
    }))
    .unwrap();

    let error = service::run(&request, &test_env()).err().unwrap();

    assert!(matches!(error, BenchmarkError::InvalidPath(_)));
    assert!(error.to_string().contains("not a directory"));
}

#[test]
fn workload_without_a_runnable_target_yields_null_runtime_metrics() {
    let temp_dir = TempDir::new().unwrap();

    let request: BenchmarkRequest = serde_json::from_value(serde_json::json!({
        "path": temp_dir.path().display().to_string(),
        "workload": {
            "method": "GET",
            "path": "/health",
            "requests": 10,
            "concurrency": 2
        }
    }))
    .unwrap();

    let result = service::run(&request, &test_env()).unwrap();

    assert_eq!(result.metrics.startup_time.status, "no_start_command");
    assert!(result.metrics.memory.is_none());
    assert!(result.metrics.http_latency.is_none());
}
