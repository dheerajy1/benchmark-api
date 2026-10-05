use benchmark_api::features::v1::benchmark::model::{
    ArtifactSize, ArtifactSizeCategory, BenchmarkRequest, BenchmarkResult, BuildCommand, BuildTime,
    Discovery, Memory, MemoryReading, Metrics, StartupReadiness, StartupTime, Target,
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
            build_time: BuildTime {
                status: "success".to_string(),
                duration_ms: Some(1842),
                command: Some(BuildCommand {
                    program: "bun".to_string(),
                    args: vec!["run".to_string(), "build".to_string()],
                }),
                working_directory: Some("/absolute/path/to/project".to_string()),
                started_at: Some("2026-10-04T12:00:00.000Z".to_string()),
                finished_at: Some("2026-10-04T12:00:01.842Z".to_string()),
                exit_code: Some(0),
            },
            startup_time: StartupTime {
                status: "success".to_string(),
                duration_ms: Some(1842),
                command: Some(BuildCommand {
                    program: "bun".to_string(),
                    args: vec!["run".to_string(), "start".to_string()],
                }),
                working_directory: Some("/absolute/path/to/project".to_string()),
                readiness: Some(StartupReadiness {
                    readiness_type: "http".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: 5001,
                    path: "/api/v1/health/app".to_string(),
                    method: "GET".to_string(),
                    expected_status: 200,
                }),
                started_at: Some("2026-10-04T12:00:00.000Z".to_string()),
                ready_at: Some("2026-10-04T12:00:01.842Z".to_string()),
                exit_code: None,
            },
            memory: Some(Memory {
                baseline: MemoryReading {
                    rss_bytes: 52428800,
                    rss_size: "50.00 MiB".to_string(),
                },
                peak: MemoryReading {
                    rss_bytes: 73400320,
                    rss_size: "70.00 MiB".to_string(),
                },
                delta: MemoryReading {
                    rss_bytes: 20971520,
                    rss_size: "20.00 MiB".to_string(),
                },
            }),
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
                },
                "build_time": {
                    "status": "success",
                    "duration_ms": 1842,
                    "command": {
                        "program": "bun",
                        "args": ["run", "build"]
                    },
                    "working_directory": "/absolute/path/to/project",
                    "started_at": "2026-10-04T12:00:00.000Z",
                    "finished_at": "2026-10-04T12:00:01.842Z",
                    "exit_code": 0
                },
                "startup_time": {
                    "status": "success",
                    "duration_ms": 1842,
                    "command": {
                        "program": "bun",
                        "args": ["run", "start"]
                    },
                    "working_directory": "/absolute/path/to/project",
                    "readiness": {
                        "type": "http",
                        "host": "127.0.0.1",
                        "port": 5001,
                        "path": "/api/v1/health/app",
                        "method": "GET",
                        "expected_status": 200
                    },
                    "started_at": "2026-10-04T12:00:00.000Z",
                    "ready_at": "2026-10-04T12:00:01.842Z",
                    "exit_code": null
                },
                "memory": {
                    "baseline": {
                        "rss_bytes": 52428800,
                        "rss_size": "50.00 MiB"
                    },
                    "peak": {
                        "rss_bytes": 73400320,
                        "rss_size": "70.00 MiB"
                    },
                    "delta": {
                        "rss_bytes": 20971520,
                        "rss_size": "20.00 MiB"
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
