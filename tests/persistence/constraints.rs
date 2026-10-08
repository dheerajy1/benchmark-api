use benchmark_api::persistence::{
    postgres::repository::PgBenchmarkRepository, repository::BenchmarkRepository,
};
use uuid::Uuid;

use super::{
    sample,
    support::{insert_bare_run, setup, sql_state},
};

const FOREIGN_KEY_VIOLATION: &str = "23503";
const RESTRICT_VIOLATION: &str = "23001";
const UNIQUE_VIOLATION: &str = "23505";
const CHECK_VIOLATION: &str = "23514";

#[tokio::test]
async fn a_run_cannot_reference_an_unknown_runtime() {
    let Some(database) = setup().await else {
        return;
    };

    let error = sqlx::query(
        "INSERT INTO benchmark_runs (id, target_path, runtime_id, created_at) \
         VALUES ($1, $2, $3, now())",
    )
    .bind(Uuid::now_v7())
    .bind("/tmp/x")
    .bind(Uuid::now_v7())
    .execute(&database.pool)
    .await
    .unwrap_err();

    assert_eq!(sql_state(&error), FOREIGN_KEY_VIOLATION);

    database.teardown().await;
}

#[tokio::test]
async fn a_referenced_runtime_cannot_be_deleted() {
    let Some(database) = setup().await else {
        return;
    };

    PgBenchmarkRepository::new(database.pool.clone())
        .save(&sample::result())
        .await
        .unwrap();

    let error = sqlx::query("DELETE FROM runtimes WHERE code = 'bun'")
        .execute(&database.pool)
        .await
        .unwrap_err();

    assert_eq!(sql_state(&error), RESTRICT_VIOLATION);

    database.teardown().await;
}

#[tokio::test]
async fn a_run_with_children_cannot_be_deleted() {
    let Some(database) = setup().await else {
        return;
    };

    let result = sample::result();

    PgBenchmarkRepository::new(database.pool.clone())
        .save(&result)
        .await
        .unwrap();

    let error = sqlx::query("DELETE FROM benchmark_runs WHERE id = $1")
        .bind(result.run_id)
        .execute(&database.pool)
        .await
        .unwrap_err();

    assert_eq!(sql_state(&error), RESTRICT_VIOLATION);

    database.teardown().await;
}

#[tokio::test]
async fn lookup_codes_are_unique() {
    let Some(database) = setup().await else {
        return;
    };

    let error =
        sqlx::query("INSERT INTO runtimes (id, code, name) VALUES ($1, 'bun', 'Bun again')")
            .bind(Uuid::now_v7())
            .execute(&database.pool)
            .await
            .unwrap_err();

    assert_eq!(sql_state(&error), UNIQUE_VIOLATION);

    database.teardown().await;
}

#[tokio::test]
async fn a_run_has_one_artifact_size_per_type() {
    let Some(database) = setup().await else {
        return;
    };

    let run_id = insert_bare_run(&database.pool).await;

    let insert = || async {
        sqlx::query(
            "INSERT INTO artifact_sizes (id, benchmark_run_id, artifact_type_id, bytes, file_count) \
             SELECT $1, $2, id, 10, 1 FROM artifact_types WHERE code = 'source'",
        )
        .bind(Uuid::now_v7())
        .bind(run_id)
        .execute(&database.pool)
        .await
    };

    insert().await.unwrap();

    let error = insert().await.unwrap_err();

    assert_eq!(sql_state(&error), UNIQUE_VIOLATION);

    database.teardown().await;
}

#[tokio::test]
async fn a_run_has_at_most_one_memory_result() {
    let Some(database) = setup().await else {
        return;
    };

    let run_id = insert_bare_run(&database.pool).await;

    let insert = || async {
        sqlx::query(
            "INSERT INTO benchmark_memories \
             (id, benchmark_run_id, baseline_rss_bytes, peak_rss_bytes, delta_rss_bytes) \
             VALUES ($1, $2, 1, 2, 1)",
        )
        .bind(Uuid::now_v7())
        .bind(run_id)
        .execute(&database.pool)
        .await
    };

    insert().await.unwrap();

    let error = insert().await.unwrap_err();

    assert_eq!(sql_state(&error), UNIQUE_VIOLATION);

    database.teardown().await;
}

#[tokio::test]
async fn negative_sizes_are_rejected() {
    let Some(database) = setup().await else {
        return;
    };

    let run_id = insert_bare_run(&database.pool).await;

    let error = sqlx::query(
        "INSERT INTO artifact_sizes (id, benchmark_run_id, artifact_type_id, bytes, file_count) \
         SELECT $1, $2, id, -1, 0 FROM artifact_types WHERE code = 'source'",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .execute(&database.pool)
    .await
    .unwrap_err();

    assert_eq!(sql_state(&error), CHECK_VIOLATION);

    database.teardown().await;
}

#[tokio::test]
async fn a_half_filled_build_is_rejected() {
    let Some(database) = setup().await else {
        return;
    };

    let run_id = insert_bare_run(&database.pool).await;

    // A program without a duration, timestamps or working directory is neither skipped nor executed
    let error = sqlx::query(
        "INSERT INTO benchmark_builds (id, benchmark_run_id, status_id, program) \
         SELECT $1, $2, id, 'bun' FROM benchmark_statuses WHERE code = 'success'",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .execute(&database.pool)
    .await
    .unwrap_err();

    assert_eq!(sql_state(&error), CHECK_VIOLATION);

    database.teardown().await;
}

#[tokio::test]
async fn a_half_filled_readiness_configuration_is_rejected() {
    let Some(database) = setup().await else {
        return;
    };

    let run_id = insert_bare_run(&database.pool).await;

    let error = sqlx::query(
        "INSERT INTO benchmark_startups (id, benchmark_run_id, status_id, readiness_host) \
         SELECT $1, $2, id, '127.0.0.1' FROM benchmark_statuses WHERE code = 'no_port'",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .execute(&database.pool)
    .await
    .unwrap_err();

    assert_eq!(sql_state(&error), CHECK_VIOLATION);

    database.teardown().await;
}

#[tokio::test]
async fn http_latency_counts_must_add_up() {
    let Some(database) = setup().await else {
        return;
    };

    let run_id = insert_bare_run(&database.pool).await;

    let error = sqlx::query(
        "INSERT INTO benchmark_http_latencies \
         (id, benchmark_run_id, status_id, total_requests, successful_requests, \
          failed_requests, duration_ms, requests_per_second) \
         SELECT $1, $2, id, 10, 5, 4, 1000, 10 FROM benchmark_statuses WHERE code = 'completed'",
    )
    .bind(Uuid::now_v7())
    .bind(run_id)
    .execute(&database.pool)
    .await
    .unwrap_err();

    assert_eq!(sql_state(&error), CHECK_VIOLATION);

    database.teardown().await;
}
