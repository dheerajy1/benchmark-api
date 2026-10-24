use benchmark_api::{
    features::v1::benchmark::configuration::WorkloadConfiguration,
    persistence::{
        postgres::repository::PgBenchmarkRepository,
        repository::{BenchmarkRepository, ConfigurationLookup},
    },
};
use sqlx::Row;
use uuid::Uuid;

use super::{sample, support::setup};

#[tokio::test]
async fn a_stored_target_configuration_is_returned_exactly() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();
    let configuration = sample::target_configuration();

    repository.save(&result, &configuration).await.unwrap();

    let lookup = repository.get_configuration(result.run_id).await.unwrap();

    assert_eq!(lookup, ConfigurationLookup::Recorded(configuration));

    database.teardown().await;
}

#[tokio::test]
async fn a_custom_configuration_without_startup_or_workload_round_trips() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();
    let configuration = sample::configuration();

    repository.save(&result, &configuration).await.unwrap();

    let lookup = repository.get_configuration(result.run_id).await.unwrap();

    assert_eq!(lookup, ConfigurationLookup::Recorded(configuration));

    database.teardown().await;
}

#[tokio::test]
async fn a_run_stored_without_a_configuration_row_is_not_recorded() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository
        .save(&result, &sample::configuration())
        .await
        .unwrap();

    // Simulates a run stored before configurations were recorded
    sqlx::query("DELETE FROM benchmark_run_configurations WHERE run_id = $1")
        .bind(result.run_id)
        .execute(&database.pool)
        .await
        .unwrap();

    let lookup = repository.get_configuration(result.run_id).await.unwrap();

    assert_eq!(lookup, ConfigurationLookup::NotRecorded);

    database.teardown().await;
}

#[tokio::test]
async fn an_unknown_run_is_reported_as_missing() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let lookup = repository.get_configuration(Uuid::now_v7()).await.unwrap();

    assert_eq!(lookup, ConfigurationLookup::RunMissing);

    database.teardown().await;
}

#[tokio::test]
async fn a_configuration_that_breaks_a_constraint_leaves_no_run_behind() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    // A requests workload without a request count violates the table's constraint
    let mut broken = sample::configuration();
    broken.workload = Some(WorkloadConfiguration {
        method: "GET".to_string(),
        path: "/".to_string(),
        mode: "requests",
        requests: None,
        duration_seconds: None,
        concurrency: 1,
        header_names: Vec::new(),
    });

    assert!(repository.save(&result, &broken).await.is_err());

    let runs: i64 = sqlx::query("SELECT COUNT(*) AS count FROM benchmark_runs WHERE id = $1")
        .bind(result.run_id)
        .fetch_one(&database.pool)
        .await
        .unwrap()
        .try_get("count")
        .unwrap();

    assert_eq!(
        runs, 0,
        "the run must not be stored without its configuration"
    );

    database.teardown().await;
}

#[tokio::test]
async fn the_listing_returns_the_newest_runs_with_their_recorded_state() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let first = sample::result();
    repository
        .save(&first, &sample::configuration())
        .await
        .unwrap();

    let second = sample::result();
    repository
        .save(&second, &sample::target_configuration())
        .await
        .unwrap();

    let listed = repository.list_configurations(10).await.unwrap();

    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|run| run.configuration.is_some()));

    let limited = repository.list_configurations(1).await.unwrap();

    assert_eq!(limited.len(), 1);

    database.teardown().await;
}
