use benchmark_api::features::v1::benchmark::model::{
    ArtifactSize, ArtifactSizeCategory, BenchmarkRequest, BenchmarkResult, BuildCommand, BuildTime,
    Cpu, Discovery, HttpLatency, Memory, MemoryReading, Metrics, StartupReadiness, StartupTime,
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
    assert!(request.startup.is_none());
    assert!(request.workload.is_none());
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
            workspaces: Vec::new(),
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
            cpu: Some(Cpu {
                baseline_percent: 0.8,
                average_percent: 12.4,
                peak_percent: 28.7,
            }),
            http_latency: Some(HttpLatency {
                status: "completed".to_string(),
                total_requests: 1000,
                successful_requests: 998,
                failed_requests: 2,
                duration_ms: 8234,
                requests_per_second: 121.45,
                min_ms: Some(1.2),
                mean_ms: Some(4.8),
                p50_ms: Some(3.9),
                p95_ms: Some(8.7),
                p99_ms: Some(15.2),
                max_ms: Some(31.4),
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
                },
                "cpu": {
                    "baseline_percent": 0.8,
                    "average_percent": 12.4,
                    "peak_percent": 28.7
                },
                "http_latency": {
                    "status": "completed",
                    "total_requests": 1000,
                    "successful_requests": 998,
                    "failed_requests": 2,
                    "duration_ms": 8234,
                    "requests_per_second": 121.45,
                    "min_ms": 1.2,
                    "mean_ms": 4.8,
                    "p50_ms": 3.9,
                    "p95_ms": 8.7,
                    "p99_ms": 15.2,
                    "max_ms": 31.4
                }
            }
        })
    );
}

#[test]
fn serializes_absent_runtime_metrics_as_null() {
    let metrics = Metrics {
        workspaces: Vec::new(),
        artifact_size: ArtifactSize {
            source: ArtifactSizeCategory {
                bytes: 0,
                size: "0 B".to_string(),
                file_count: 0,
            },
            build: ArtifactSizeCategory {
                bytes: 0,
                size: "0 B".to_string(),
                file_count: 0,
            },
            excluded: ArtifactSizeCategory {
                bytes: 0,
                size: "0 B".to_string(),
                file_count: 0,
            },
        },
        build_time: BuildTime {
            status: "no_build_command".to_string(),
            duration_ms: None,
            command: None,
            working_directory: None,
            started_at: None,
            finished_at: None,
            exit_code: None,
        },
        startup_time: StartupTime {
            status: "no_start_command".to_string(),
            duration_ms: None,
            command: None,
            working_directory: None,
            readiness: None,
            started_at: None,
            ready_at: None,
            exit_code: None,
        },
        memory: None,
        cpu: None,
        http_latency: None,
    };

    let value: Value = serde_json::to_value(metrics).unwrap();

    assert_eq!(value["memory"], Value::Null);
    assert_eq!(value["cpu"], Value::Null);
    assert_eq!(value["http_latency"], Value::Null);
}

#[test]
fn generates_uuidv7_run_id() {
    let uuid = Uuid::now_v7();

    assert_eq!(uuid.get_version_num(), 7);
}
