use super::sample::configuration;
use benchmark_api::persistence::{
    error::PersistenceError,
    postgres::{mapping::run_created_at, repository::PgBenchmarkRepository},
    repository::BenchmarkRepository,
};
use chrono::{DateTime, Utc};
use sqlx::types::Json;
use uuid::Uuid;

use super::{
    sample,
    support::{count_for_run, setup},
};

const RUN_TABLES: [&str; 7] = [
    "benchmark_runs",
    "artifact_sizes",
    "benchmark_builds",
    "benchmark_startups",
    "benchmark_memories",
    "benchmark_cpus",
    "benchmark_http_latencies",
];

#[tokio::test]
async fn saves_a_complete_result_across_every_table() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository.save(&result, &configuration()).await.unwrap();

    let run_id = result.run_id;

    assert_eq!(
        count_for_run(&database.pool, "benchmark_runs", run_id).await,
        1
    );
    assert_eq!(
        count_for_run(&database.pool, "artifact_sizes", run_id).await,
        3
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_builds", run_id).await,
        1
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_startups", run_id).await,
        1
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_memories", run_id).await,
        1
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_cpus", run_id).await,
        1
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_http_latencies", run_id).await,
        1
    );

    database.teardown().await;
}

#[tokio::test]
async fn stores_canonical_values() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository.save(&result, &configuration()).await.unwrap();

    let run_id = result.run_id;

    let runtime = sqlx::query_scalar::<_, String>(
        "SELECT r.code FROM benchmark_runs b JOIN runtimes r ON r.id = b.runtime_id WHERE b.id = $1",
    )
    .bind(run_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();

    assert_eq!(runtime, "bun");

    let created_at = sqlx::query_scalar::<_, DateTime<Utc>>(
        "SELECT created_at FROM benchmark_runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();

    assert_eq!(created_at, run_created_at(run_id).unwrap());

    let args = sqlx::query_scalar::<_, Json<Vec<String>>>(
        "SELECT args FROM benchmark_builds WHERE benchmark_run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();

    assert_eq!(args.0, vec!["run".to_string(), "build".to_string()]);

    let started_at = sqlx::query_scalar::<_, DateTime<Utc>>(
        "SELECT started_at FROM benchmark_builds WHERE benchmark_run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();

    assert_eq!(started_at.timestamp_subsec_millis(), 0);
    assert_eq!(started_at.to_rfc3339(), "2026-10-04T12:00:00+00:00");

    let requests_per_second = sqlx::query_scalar::<_, f64>(
        "SELECT requests_per_second::float8 FROM benchmark_http_latencies WHERE benchmark_run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();

    assert_eq!(requests_per_second, 121.45);

    let peak = sqlx::query_scalar::<_, i64>(
        "SELECT peak_rss_bytes FROM benchmark_memories WHERE benchmark_run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();

    assert_eq!(peak, 73_400_320);

    database.teardown().await;
}

#[tokio::test]
async fn saves_a_run_without_runtime_and_without_workload_metrics() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::skipped_result();

    repository.save(&result, &configuration()).await.unwrap();

    let run_id = result.run_id;

    let runtime_id = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT runtime_id FROM benchmark_runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();

    assert_eq!(runtime_id, None);

    assert_eq!(
        count_for_run(&database.pool, "artifact_sizes", run_id).await,
        3
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_builds", run_id).await,
        1
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_startups", run_id).await,
        1
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_memories", run_id).await,
        0
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_cpus", run_id).await,
        0
    );
    assert_eq!(
        count_for_run(&database.pool, "benchmark_http_latencies", run_id).await,
        0
    );

    database.teardown().await;
}

#[tokio::test]
async fn nothing_is_stored_when_a_late_insert_fails() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let mut result = sample::result();

    // The HTTP latency is inserted last, so everything before it must roll back
    if let Some(http_latency) = result.metrics.http_latency.as_mut() {
        http_latency.status = "bogus".to_string();
    }

    let error = repository
        .save(&result, &configuration())
        .await
        .unwrap_err();

    assert_eq!(
        error,
        PersistenceError::UnknownLookup {
            table: "benchmark_statuses",
            code: "bogus".to_string(),
        }
    );

    for table in RUN_TABLES {
        assert_eq!(
            count_for_run(&database.pool, table, result.run_id).await,
            0,
            "{table} must be empty after the rollback"
        );
    }

    database.teardown().await;
}

#[tokio::test]
async fn an_unknown_runtime_is_rejected_and_nothing_is_stored() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let mut result = sample::result();

    result.discovery.runtime = Some("ruby".to_string());

    let error = repository
        .save(&result, &configuration())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        PersistenceError::UnknownLookup {
            table: "runtimes",
            ..
        }
    ));

    assert_eq!(
        count_for_run(&database.pool, "benchmark_runs", result.run_id).await,
        0
    );

    database.teardown().await;
}

#[tokio::test]
async fn saving_the_same_run_twice_fails_and_keeps_the_first_copy_intact() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository.save(&result, &configuration()).await.unwrap();

    let error = repository
        .save(&result, &configuration())
        .await
        .unwrap_err();

    assert!(matches!(error, PersistenceError::Database(_)));

    assert_eq!(
        count_for_run(&database.pool, "benchmark_runs", result.run_id).await,
        1
    );
    assert_eq!(
        count_for_run(&database.pool, "artifact_sizes", result.run_id).await,
        3
    );

    database.teardown().await;
}
