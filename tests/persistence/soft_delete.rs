use benchmark_api::persistence::{
    history::{SummaryQuery, TrendQuery},
    postgres::repository::PgBenchmarkRepository,
    repository::{BenchmarkRepository, ConfigurationLookup},
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{
    sample::{self, configuration},
    support::{count_for_run, setup},
};

fn query() -> SummaryQuery {
    SummaryQuery {
        limit: 50,
        ..SummaryQuery::default()
    }
}

async fn save(repository: &PgBenchmarkRepository, path: &str) -> Uuid {
    let mut result = sample::result();

    result.target.path = path.to_string();

    repository.save(&result, &configuration()).await.unwrap();

    result.run_id
}

#[tokio::test]
async fn a_deleted_run_is_in_no_list_count_detail_trend_or_configuration() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let kept = save(&repository, "/work/kept").await;
    let deleted = save(&repository, "/work/deleted").await;

    assert!(repository.delete_run(deleted).await.unwrap());

    let listed = repository.list_summaries(&query()).await.unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, kept);

    let counts = repository.count_history(&query()).await.unwrap();

    assert_eq!(counts.total, 1);
    assert_eq!(counts.by_runtime, vec![("bun".to_string(), 1)]);

    let searched = repository
        .list_summaries(&SummaryQuery {
            search: Some("deleted".to_string()),
            ..query()
        })
        .await
        .unwrap();

    assert!(searched.is_empty());

    assert_eq!(repository.count_runs().await.unwrap(), 1);

    let trends = repository.list_trends(&TrendQuery::all(30)).await.unwrap();

    assert_eq!(trends.len(), 1);
    assert_eq!(trends[0].id, kept);

    assert!(repository.get_by_run_id(deleted).await.unwrap().is_none());

    let both = repository.get_runs(&[kept, deleted]).await.unwrap();

    assert_eq!(both.len(), 1);
    assert_eq!(both[0].result.run_id, kept);

    assert_eq!(
        repository.get_configuration(deleted).await.unwrap(),
        ConfigurationLookup::RunMissing
    );

    let configurations = repository.list_configurations(50).await.unwrap();

    assert_eq!(configurations.len(), 1);
    assert_eq!(configurations[0].run_id, kept);

    database.teardown().await;
}

#[tokio::test]
async fn deleting_keeps_every_part_row_so_a_restore_is_one_update() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let run_id = save(&repository, "/work/kept-rows").await;

    repository.delete_run(run_id).await.unwrap();

    for table in [
        "benchmark_runs",
        "benchmark_builds",
        "benchmark_startups",
        "benchmark_memories",
        "benchmark_cpus",
        "benchmark_http_latencies",
        "artifact_sizes",
        "benchmark_run_configurations",
        "benchmark_run_stages",
    ] {
        let rows = if table == "benchmark_run_configurations" {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM benchmark_run_configurations WHERE run_id = $1",
            )
            .bind(run_id)
            .fetch_one(&database.pool)
            .await
            .unwrap()
        } else {
            count_for_run(&database.pool, table, run_id).await
        };

        assert!(rows > 0, "{table} lost its rows");
    }

    sqlx::query("UPDATE benchmark_runs SET deleted_at = NULL WHERE id = $1")
        .bind(run_id)
        .execute(&database.pool)
        .await
        .unwrap();

    assert_eq!(repository.list_summaries(&query()).await.unwrap().len(), 1);

    database.teardown().await;
}

#[tokio::test]
async fn deleting_twice_is_fine_and_keeps_the_first_deletion_time() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let run_id = save(&repository, "/work/twice").await;

    let deleted_at = |pool: sqlx::PgPool| async move {
        sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            "SELECT deleted_at FROM benchmark_runs WHERE id = $1",
        )
        .bind(run_id)
        .fetch_one(&pool)
        .await
        .unwrap()
    };

    assert!(deleted_at(database.pool.clone()).await.is_none());

    assert!(repository.delete_run(run_id).await.unwrap());

    let first = deleted_at(database.pool.clone()).await;

    assert!(first.is_some());

    assert!(repository.delete_run(run_id).await.unwrap());
    assert_eq!(deleted_at(database.pool.clone()).await, first);

    database.teardown().await;
}

#[tokio::test]
async fn deleting_a_run_that_was_never_stored_reports_it_missing() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    assert!(!repository.delete_run(Uuid::now_v7()).await.unwrap());

    database.teardown().await;
}
