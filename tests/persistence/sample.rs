use benchmark_api::features::v1::benchmark::model::{
    ArtifactSize, ArtifactSizeCategory, BenchmarkResult, BuildCommand, BuildTime, Cpu, Discovery,
    HttpLatency, Memory, MemoryReading, Metrics, StartupReadiness, StartupTime, Target,
};
use uuid::Uuid;

// A complete benchmark result with every metric present and the runtime detected
pub fn result() -> BenchmarkResult {
    BenchmarkResult {
        run_id: Uuid::now_v7(),
        target: Target {
            path: "/absolute/path/to/project".to_string(),
        },
        discovery: Discovery {
            runtime: Some("bun".to_string()),
            build: Some("bun run build".to_string()),
        },
        metrics: Metrics {
            artifact_size: ArtifactSize {
                source: category(100, 1),
                build: category(200, 2),
                excluded: category(300, 3),
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
                duration_ms: Some(1500),
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
                started_at: Some("2026-10-04T12:00:02.000Z".to_string()),
                ready_at: Some("2026-10-04T12:00:03.500Z".to_string()),
                exit_code: None,
            },
            memory: Some(Memory {
                baseline: reading(52_428_800),
                peak: reading(73_400_320),
                delta: reading(20_971_520),
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
    }
}

// A run where no build command, no start command and no workload existed
pub fn skipped_result() -> BenchmarkResult {
    let mut result = result();

    result.discovery.runtime = None;
    result.discovery.build = None;

    result.metrics.build_time = BuildTime {
        status: "no_build_command".to_string(),
        duration_ms: None,
        command: None,
        working_directory: None,
        started_at: None,
        finished_at: None,
        exit_code: None,
    };

    result.metrics.startup_time = StartupTime {
        status: "no_start_command".to_string(),
        duration_ms: None,
        command: None,
        working_directory: None,
        readiness: None,
        started_at: None,
        ready_at: None,
        exit_code: None,
    };

    result.metrics.memory = None;
    result.metrics.cpu = None;
    result.metrics.http_latency = None;

    result
}

fn category(bytes: u64, file_count: u64) -> ArtifactSizeCategory {
    ArtifactSizeCategory {
        bytes,
        size: format!("{bytes} B"),
        file_count,
    }
}

fn reading(rss_bytes: u64) -> MemoryReading {
    MemoryReading {
        rss_bytes,
        rss_size: format!("{rss_bytes} B"),
    }
}
