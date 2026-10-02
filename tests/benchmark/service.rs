use std::fs::{self, File};

use benchmark_api::features::v1::benchmark::{model::BenchmarkRequest, service};
use tempfile::TempDir;
use uuid::Uuid;

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
    };

    let result = service::run(&request).unwrap();

    assert_eq!(result.run_id.get_version_num(), 7);
    assert_eq!(result.target.path, temp_dir.path().display().to_string());

    assert!(result.discovery.runtime.is_none());
    assert!(result.discovery.build.is_none());

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
