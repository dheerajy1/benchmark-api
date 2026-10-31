use benchmark_api::features::v1::benchmark::model::{
    ArtifactSize, ArtifactSizeCategory, BenchmarkResult, BuildCommand, BuildTime, Cpu, Discovery,
    HttpLatency, Memory, MemoryReading, Metrics, StartupReadiness, StartupTime, Target,
};
use uuid::Uuid;

// A complete run with round numbers, every metric present and every step successful
pub fn result(run_id: Uuid) -> BenchmarkResult {
    BenchmarkResult {
        run_id,
        target: Target {
            path: "/projects/api".to_string(),
        },
        discovery: Discovery {
            runtime: Some("bun".to_string()),
            build: None,
        },
        metrics: Metrics {
            workspaces: Vec::new(),
            artifact_size: ArtifactSize {
                source: category(1000, 10),
                build: category(2000, 20),
                excluded: category(3000, 30),
            },
            build_time: BuildTime {
                status: "success".to_string(),
                duration_ms: Some(1000),
                command: Some(command("build")),
                working_directory: Some("/projects/api".to_string()),
                started_at: Some("2026-10-04T12:00:00.000Z".to_string()),
                finished_at: Some("2026-10-04T12:00:01.000Z".to_string()),
                exit_code: Some(0),
            },
            startup_time: StartupTime {
                status: "success".to_string(),
                duration_ms: Some(500),
                command: Some(command("start")),
                working_directory: Some("/projects/api".to_string()),
                readiness: Some(StartupReadiness {
                    readiness_type: "http".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: 5001,
                    path: "/health".to_string(),
                    method: "GET".to_string(),
                    expected_status: 200,
                }),
                started_at: Some("2026-10-04T12:00:02.000Z".to_string()),
                ready_at: Some("2026-10-04T12:00:02.500Z".to_string()),
                exit_code: None,
            },
            memory: Some(Memory {
                baseline: reading(100),
                peak: reading(200),
                delta: reading(100),
            }),
            cpu: Some(Cpu {
                baseline_percent: 10.0,
                average_percent: 20.0,
                peak_percent: 40.0,
            }),
            http_latency: Some(HttpLatency {
                status: "completed".to_string(),
                total_requests: 1000,
                successful_requests: 1000,
                failed_requests: 0,
                duration_ms: 10_000,
                requests_per_second: 100.0,
                min_ms: Some(1.0),
                mean_ms: Some(12.0),
                p50_ms: Some(10.0),
                p95_ms: Some(20.0),
                p99_ms: Some(40.0),
                max_ms: Some(50.0),
            }),
        },
    }
}

// A run where nothing could be built or started and no workload existed
pub fn skipped(run_id: Uuid) -> BenchmarkResult {
    let mut run = result(run_id);

    run.discovery.runtime = None;

    run.metrics.build_time = BuildTime {
        status: "no_build_command".to_string(),
        duration_ms: None,
        command: None,
        working_directory: None,
        started_at: None,
        finished_at: None,
        exit_code: None,
    };

    run.metrics.startup_time = StartupTime {
        status: "no_start_command".to_string(),
        duration_ms: None,
        command: None,
        working_directory: None,
        readiness: None,
        started_at: None,
        ready_at: None,
        exit_code: None,
    };

    run.metrics.memory = None;
    run.metrics.cpu = None;
    run.metrics.http_latency = None;

    run
}

fn command(script: &str) -> BuildCommand {
    BuildCommand {
        program: "bun".to_string(),
        args: vec!["run".to_string(), script.to_string()],
    }
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
