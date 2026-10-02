use benchmark_api::features::v1::benchmark::model::{
    ArtifactSize, ArtifactSizeCategory, BenchmarkRequest, BenchmarkResult, Discovery, Metrics,
    Target,
};
use serde_json::{Value, json};
use uuid::Uuid;

#[test]
fn deserializes_benchmark_request() {
    let request: BenchmarkRequest =
        serde_json::from_str(r#"{"path":"~/dev/vs-code/recordlog-api"}"#).unwrap();

    assert_eq!(
        request.path.to_string_lossy(),
        "~/dev/vs-code/recordlog-api"
    );
}

#[test]
fn serializes_benchmark_result() {
    let result = BenchmarkResult {
        run_id: Uuid::now_v7(),
        target: Target {
            path: "/absolute/path/to/project".to_string(),
        },
        discovery: Discovery {
            runtime: None,
            build: None,
        },
        metrics: Metrics {
            artifact_size: ArtifactSize {
                source: ArtifactSizeCategory {
                    bytes: 100,
                    size: "100 B".to_string(),
                    file_count: 1,
                },
                build: ArtifactSizeCategory {
                    bytes: 200,
                    size: "200 B".to_string(),
                    file_count: 2,
                },
                excluded: ArtifactSizeCategory {
                    bytes: 300,
                    size: "300 B".to_string(),
                    file_count: 3,
                },
            },
        },
    };

    let value: Value = serde_json::to_value(result).unwrap();

    assert_eq!(
        value,
        json!({
            "run_id": value["run_id"],
            "target": {
                "path": "/absolute/path/to/project"
            },
            "discovery": {
                "runtime": null,
                "build": null
            },
            "metrics": {
                "artifact_size": {
                    "source": {
                        "bytes": 100,
                        "size": "100 B",
                        "file_count": 1
                    },
                    "build": {
                        "bytes": 200,
                        "size": "200 B",
                        "file_count": 2
                    },
                    "excluded": {
                        "bytes": 300,
                        "size": "300 B",
                        "file_count": 3
                    }
                }
            }
        })
    );
}

#[test]
fn generates_uuidv7_run_id() {
    let uuid = Uuid::now_v7();

    assert_eq!(uuid.get_version_num(), 7);
}
