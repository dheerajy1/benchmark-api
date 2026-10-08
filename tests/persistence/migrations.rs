use benchmark_api::persistence::postgres::connection::MIGRATOR;
use sqlx::PgPool;
use uuid::Uuid;

use super::support::setup;

async fn codes(pool: &PgPool, table: &str) -> Vec<String> {
    let sql = format!("SELECT code FROM {table} ORDER BY code");

    sqlx::query_scalar::<_, String>(&sql)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn seeds_every_runtime_discovery_can_report() {
    let Some(database) = setup().await else {
        return;
    };

    assert_eq!(
        codes(&database.pool, "runtimes").await,
        vec!["bun", "go", "node", "python", "rust"]
    );

    database.teardown().await;
}

#[tokio::test]
async fn seeds_the_artifact_types() {
    let Some(database) = setup().await else {
        return;
    };

    assert_eq!(
        codes(&database.pool, "artifact_types").await,
        vec!["build", "excluded", "source"]
    );

    database.teardown().await;
}

#[tokio::test]
async fn seeds_every_status_the_benchmark_code_emits() {
    let Some(database) = setup().await else {
        return;
    };

    assert_eq!(
        codes(&database.pool, "benchmark_statuses").await,
        vec![
            "completed",
            "failed",
            "no_build_command",
            "no_port",
            "no_start_command",
            "success",
            "target_exited",
            "timeout",
            "unhealthy",
        ]
    );

    database.teardown().await;
}

#[tokio::test]
async fn lookup_ids_are_uuid_v7() {
    let Some(database) = setup().await else {
        return;
    };

    for table in ["runtimes", "artifact_types", "benchmark_statuses"] {
        let sql = format!("SELECT id FROM {table}");

        let ids = sqlx::query_scalar::<_, Uuid>(&sql)
            .fetch_all(&database.pool)
            .await
            .unwrap();

        assert!(!ids.is_empty());
        assert!(ids.iter().all(|id| id.get_version_num() == 7));
    }

    database.teardown().await;
}

#[tokio::test]
async fn running_the_migrations_again_changes_nothing() {
    let Some(database) = setup().await else {
        return;
    };

    MIGRATOR.run(&database.pool).await.unwrap();

    assert_eq!(codes(&database.pool, "benchmark_statuses").await.len(), 9);

    database.teardown().await;
}

#[tokio::test]
async fn the_schema_uses_no_enum_types() {
    let Some(database) = setup().await else {
        return;
    };

    let enums = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pg_type t \
         JOIN pg_namespace n ON n.oid = t.typnamespace \
         WHERE t.typtype = 'e' AND n.nspname = current_schema()",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();

    assert_eq!(enums, 0);

    database.teardown().await;
}

#[tokio::test]
async fn creates_the_planned_indexes() {
    let Some(database) = setup().await else {
        return;
    };

    let indexes = sqlx::query_scalar::<_, String>(
        "SELECT indexname FROM pg_indexes WHERE schemaname = current_schema()",
    )
    .fetch_all(&database.pool)
    .await
    .unwrap();

    for expected in [
        "idx_benchmark_runs_created_at",
        "idx_benchmark_runs_runtime_id_created_at",
        "idx_benchmark_runs_target_path_created_at",
        "idx_artifact_sizes_artifact_type_id",
    ] {
        assert!(
            indexes.iter().any(|index| index == expected),
            "missing index {expected}"
        );
    }

    database.teardown().await;
}
