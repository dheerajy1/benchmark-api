use benchmark_api::{
    features::v1::benchmark::{
        configuration::WorkspaceConfiguration,
        model::{Cpu, Memory, MemoryReading},
        workspace::result::{
            ImageSize, LoadResult, WorkspaceBuild, WorkspaceResult, WorkspaceStartup,
        },
    },
    persistence::{
        postgres::repository::PgBenchmarkRepository,
        repository::{BenchmarkRepository, ConfigurationLookup},
    },
};

use super::{
    sample,
    support::{count_for_run, setup},
};

fn reading(bytes: u64) -> MemoryReading {
    MemoryReading {
        rss_bytes: bytes,
        rss_size: format!("{bytes} B"),
    }
}

fn docker_result() -> WorkspaceResult {
    let mut result = WorkspaceResult::empty("subscriber", "docker", vec!["TOKEN".to_string()]);

    result.status = "success".to_string();
    result.build = Some(WorkspaceBuild {
        status: "success".to_string(),
        duration_ms: 90_000,
        exit_code: Some(0),
    });
    result.image = Some(ImageSize {
        bytes: 123_456_789,
        size: "117.74 MB".to_string(),
        disk_usage_bytes: Some(276_000_000),
        disk_usage: Some("263.21 MB".to_string()),
    });
    result.startup = Some(WorkspaceStartup { duration_ms: 812 });
    result.memory = Some(Memory {
        baseline: reading(1000),
        peak: reading(1500),
        delta: reading(500),
    });
    result.cpu = Some(Cpu {
        baseline_percent: 0.5,
        average_percent: 10.25,
        peak_percent: 20.0,
    });
    result.load = Some(LoadResult {
        status: "success".to_string(),
        exit_code: Some(0),
        duration_ms: 5000,
    });

    result
}

fn failed_native() -> WorkspaceResult {
    let mut result = WorkspaceResult::empty("subscriber", "native", vec![]);

    result.status = "timeout".to_string();
    result.message = Some("The ready line was not seen within 60 seconds".to_string());

    result
}

#[tokio::test]
async fn workspace_results_round_trip_in_order_with_null_numbers_kept() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let mut result = sample::result();

    result.metrics.workspaces = vec![failed_native(), docker_result()];

    repository
        .save(&result, &sample::configuration())
        .await
        .unwrap();

    assert_eq!(
        count_for_run(&database.pool, "benchmark_workspace_results", result.run_id).await,
        2
    );

    let stored = repository
        .get_by_run_id(result.run_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(stored.metrics.workspaces, result.metrics.workspaces);

    database.teardown().await;
}

#[tokio::test]
async fn a_run_without_workspaces_stores_no_workspace_rows() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository
        .save(&result, &sample::configuration())
        .await
        .unwrap();

    assert_eq!(
        count_for_run(&database.pool, "benchmark_workspace_results", result.run_id).await,
        0
    );

    let stored = repository
        .get_by_run_id(result.run_id)
        .await
        .unwrap()
        .unwrap();

    assert!(stored.metrics.workspaces.is_empty());

    database.teardown().await;
}

#[tokio::test]
async fn the_workspace_configuration_round_trips_with_names_only() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();
    let mut configuration = sample::configuration();

    configuration.workspaces = vec![WorkspaceConfiguration {
        name: "subscriber".to_string(),
        directory: "subscriber".to_string(),
        env_source: Some("pasted".to_string()),
        env_file_path: None,
        env_variable_names: vec!["TOKEN".to_string()],
        ready_log_line: "subscriber ready".to_string(),
        ready_timeout_seconds: 60,
        load_command: None,
        idle_seconds: 30,
        native: None,
        docker: None,
    }];

    repository.save(&result, &configuration).await.unwrap();

    let lookup = repository.get_configuration(result.run_id).await.unwrap();

    assert_eq!(
        lookup,
        ConfigurationLookup::Recorded(Box::new(configuration))
    );

    database.teardown().await;
}
