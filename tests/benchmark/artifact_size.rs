use std::fs::{self, File};

use benchmark_api::features::v1::benchmark::metrics::artifact_size;
use tempfile::TempDir;

#[test]
fn measures_source_build_and_excluded_bytes_and_counts() {
    let temp_dir = TempDir::new().unwrap();

    let source = temp_dir.path().join("src.txt");
    let target = temp_dir.path().join("target");
    let build = target.join("debug");
    let node_modules = temp_dir.path().join("node_modules");

    fs::create_dir_all(&build).unwrap();
    fs::create_dir_all(&node_modules).unwrap();

    File::create(&source).unwrap().set_len(100).unwrap();

    File::create(build.join("artifact"))
        .unwrap()
        .set_len(200)
        .unwrap();

    File::create(node_modules.join("dependency"))
        .unwrap()
        .set_len(300)
        .unwrap();

    let result = artifact_size::measure(temp_dir.path(), "target").unwrap();

    assert_eq!(result.source.bytes, 100);
    assert_eq!(result.source.file_count, 1);

    assert_eq!(result.build.bytes, 200);
    assert_eq!(result.build.file_count, 1);

    assert_eq!(result.excluded.bytes, 300);
    assert_eq!(result.excluded.file_count, 1);
}

#[test]
fn build_classification_takes_precedence_over_exclusion() {
    let temp_dir = TempDir::new().unwrap();

    let build = temp_dir.path().join("target").join("node_modules");
    fs::create_dir_all(&build).unwrap();

    File::create(build.join("artifact"))
        .unwrap()
        .set_len(500)
        .unwrap();

    let result = artifact_size::measure(temp_dir.path(), "target").unwrap();

    assert_eq!(result.build.bytes, 500);
    assert_eq!(result.build.file_count, 1);

    assert_eq!(result.excluded.bytes, 0);
    assert_eq!(result.excluded.file_count, 0);

    assert_eq!(result.source.bytes, 0);
    assert_eq!(result.source.file_count, 0);
}

#[test]
fn formats_binary_sizes() {
    let temp_dir = TempDir::new().unwrap();

    File::create(temp_dir.path().join("source"))
        .unwrap()
        .set_len(2_621_440)
        .unwrap();

    fs::create_dir(temp_dir.path().join("dist")).unwrap();

    let result = artifact_size::measure(temp_dir.path(), "dist").unwrap();

    assert_eq!(result.source.size, "2.50 MiB");
}

#[test]
fn formats_bytes_and_kib() {
    let temp_dir = TempDir::new().unwrap();

    File::create(temp_dir.path().join("bytes"))
        .unwrap()
        .set_len(512)
        .unwrap();

    fs::create_dir(temp_dir.path().join("dist")).unwrap();

    let result = artifact_size::measure(temp_dir.path(), "dist").unwrap();

    assert_eq!(result.source.size, "512 B");

    let temp_dir = TempDir::new().unwrap();

    File::create(temp_dir.path().join("kib"))
        .unwrap()
        .set_len(2048)
        .unwrap();

    fs::create_dir(temp_dir.path().join("dist")).unwrap();

    let result = artifact_size::measure(temp_dir.path(), "dist").unwrap();

    assert_eq!(result.source.size, "2.00 KiB");
}

#[test]
fn only_the_named_output_folder_is_the_build() {
    let temp_dir = TempDir::new().unwrap();

    let release = temp_dir.path().join("target").join("release");
    let debug = temp_dir.path().join("target").join("debug");

    fs::create_dir_all(&release).unwrap();
    fs::create_dir_all(&debug).unwrap();

    File::create(release.join("app"))
        .unwrap()
        .set_len(400)
        .unwrap();
    File::create(debug.join("app"))
        .unwrap()
        .set_len(900)
        .unwrap();
    File::create(temp_dir.path().join("main.rs"))
        .unwrap()
        .set_len(50)
        .unwrap();

    let result = artifact_size::measure(temp_dir.path(), "./target/release").unwrap();

    assert_eq!(result.build.bytes, 400);
    assert_eq!(result.excluded.bytes, 900);
    assert_eq!(result.source.bytes, 50);
}

#[test]
fn a_missing_output_folder_is_an_error_that_names_it() {
    let temp_dir = TempDir::new().unwrap();

    let error = artifact_size::measure(temp_dir.path(), "dist")
        .err()
        .unwrap();

    assert!(error.contains("dist"), "{error}");
}

#[test]
fn the_default_folder_depends_on_the_runtime() {
    use benchmark_api::features::v1::benchmark::model::BuildRequest;

    let none = BuildRequest {
        command: "build".to_string(),
        output_dir: None,
    };
    let named = BuildRequest {
        command: "build".to_string(),
        output_dir: Some(" target/release ".to_string()),
    };

    assert_eq!(artifact_size::output_dir(&none, Some("rust")), "target");
    assert_eq!(artifact_size::output_dir(&none, Some("bun")), "dist");
    assert_eq!(artifact_size::output_dir(&none, None), "dist");
    assert_eq!(
        artifact_size::output_dir(&named, Some("rust")),
        "target/release"
    );
}
