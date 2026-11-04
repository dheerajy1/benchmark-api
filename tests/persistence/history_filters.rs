use benchmark_api::{
    features::v1::benchmark::{model::BenchmarkResult, workspace::result::WorkspaceResult},
    persistence::{
        history::{HistoryCounts, SummaryQuery},
        outcome::{RunOutcome, StoredStatus},
        postgres::repository::PgBenchmarkRepository,
        repository::BenchmarkRepository,
    },
};
use uuid::Uuid;

use super::{
    sample::{self, configuration},
    support::setup,
};

fn query() -> SummaryQuery {
    SummaryQuery {
        limit: 50,
        ..SummaryQuery::default()
    }
}

fn run_at(path: &str, runtime: Option<&str>) -> BenchmarkResult {
    let mut result = sample::result();

    result.target.path = path.to_string();
    result.discovery.runtime = runtime.map(str::to_string);

    result
}

// A complete run, a failed run and a cancelled run, in three paths and three runtimes
async fn seed(repository: &PgBenchmarkRepository) -> (Uuid, Uuid, Uuid) {
    let done = run_at("/work/Billing-API", Some("bun"));
    let failed = BenchmarkResult::empty(Uuid::now_v7(), "/work/ledger".to_string());
    let mut cancelled = run_at("/work/ledger", None);

    cancelled.metrics.http_latency = None;
    cancelled.metrics.memory = None;
    cancelled.metrics.cpu = None;

    repository.save(&done, &configuration()).await.unwrap();

    repository
        .save_run(
            &failed,
            &configuration(),
            RunOutcome::failed(
                Some("discovery".to_string()),
                "No runtime was found.".to_string(),
                Vec::new(),
            ),
        )
        .await
        .unwrap();

    repository
        .save_run(
            &cancelled,
            &configuration(),
            RunOutcome::cancelled(Some("workload".to_string()), Vec::new()),
        )
        .await
        .unwrap();

    (done.run_id, failed.run_id, cancelled.run_id)
}

fn count_of(counts: &HistoryCounts, status: StoredStatus) -> i64 {
    counts
        .by_status
        .iter()
        .find(|(known, _)| *known == status)
        .map(|(_, runs)| *runs)
        .unwrap()
}

#[tokio::test]
async fn the_status_filter_lists_only_that_status_and_the_counts_ignore_it() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let (_, failed, _) = seed(&repository).await;

    let filtered = SummaryQuery {
        status: Some(StoredStatus::Failed),
        ..query()
    };

    let rows = repository.list_summaries(&filtered).await.unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, failed);
    assert_eq!(rows[0].status, StoredStatus::Failed);

    let counts = repository.count_history(&filtered).await.unwrap();

    // The total follows every filter, the status pills follow all but the status
    assert_eq!(counts.total, 1);
    assert_eq!(count_of(&counts, StoredStatus::Done), 1);
    assert_eq!(count_of(&counts, StoredStatus::Failed), 1);
    assert_eq!(count_of(&counts, StoredStatus::Cancelled), 1);

    database.teardown().await;
}

#[tokio::test]
async fn the_runtime_counts_follow_the_other_filters_but_not_the_runtime() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    seed(&repository).await;

    let counts = repository.count_history(&query()).await.unwrap();

    assert_eq!(counts.total, 3);
    assert_eq!(
        counts.by_runtime,
        vec![("bun".to_string(), 1), ("unknown".to_string(), 2)]
    );

    // A runtime filter does not change the runtime pills, a status filter does
    let counts = repository
        .count_history(&SummaryQuery {
            runtime_code: Some("bun".to_string()),
            status: Some(StoredStatus::Cancelled),
            ..query()
        })
        .await
        .unwrap();

    assert_eq!(counts.total, 0);
    assert_eq!(counts.by_runtime, vec![("unknown".to_string(), 1)]);

    database.teardown().await;
}

#[tokio::test]
async fn the_search_matches_the_path_in_any_case_and_the_id_by_suffix_or_part() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let (done, failed, _) = seed(&repository).await;

    let search = |text: &str| SummaryQuery {
        search: Some(text.to_string()),
        ..query()
    };

    let by_path = repository
        .list_summaries(&search("billing-api"))
        .await
        .unwrap();

    assert_eq!(by_path.len(), 1);
    assert_eq!(by_path[0].id, done);

    // The short id is the last 8 characters of the id
    let id = failed.to_string();
    let by_short_id = repository
        .list_summaries(&search(&id[id.len() - 8..].to_uppercase()))
        .await
        .unwrap();

    assert_eq!(by_short_id.len(), 1);
    assert_eq!(by_short_id[0].id, failed);

    let by_middle = repository
        .list_summaries(&search(&id[20..30]))
        .await
        .unwrap();

    assert_eq!(by_middle.len(), 1);
    assert_eq!(by_middle[0].id, failed);

    let counts = repository.count_history(&search("ledger")).await.unwrap();

    assert_eq!(counts.total, 2);

    database.teardown().await;
}

#[tokio::test]
async fn wildcards_in_the_search_text_are_matched_as_typed() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    seed(&repository).await;

    for text in ["%", "_", "\\", "led_er", "b%g"] {
        let rows = repository
            .list_summaries(&SummaryQuery {
                search: Some(text.to_string()),
                ..query()
            })
            .await
            .unwrap();

        assert!(
            rows.is_empty(),
            "search '{text}' matched {} runs",
            rows.len()
        );
    }

    database.teardown().await;
}

#[tokio::test]
async fn the_search_works_with_the_other_filters_and_the_cursor() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let (_, failed, cancelled) = seed(&repository).await;

    let both = SummaryQuery {
        search: Some("ledger".to_string()),
        ..query()
    };

    let rows = repository.list_summaries(&both).await.unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].id, rows[1].id), (cancelled, failed));

    let narrowed = SummaryQuery {
        status: Some(StoredStatus::Failed),
        ..both.clone()
    };

    let rows = repository.list_summaries(&narrowed).await.unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, failed);

    let after_cancelled = SummaryQuery {
        after: Some(benchmark_api::persistence::history::CursorPosition {
            created_at: rows[0].created_at,
            id: failed,
        }),
        ..both
    };

    assert!(
        repository
            .list_summaries(&after_cancelled)
            .await
            .unwrap()
            .is_empty()
    );

    database.teardown().await;
}

#[tokio::test]
async fn the_sizes_and_workspaces_are_listed_and_missing_ones_are_null() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let mut with_docker = run_at("/work/docker-app", Some("bun"));
    let mut docker = WorkspaceResult::empty("subscriber", "docker", Vec::new());

    docker.status = "success".to_string();
    docker.image = Some(
        benchmark_api::features::v1::benchmark::workspace::result::ImageSize {
            bytes: 1000,
            size: "1000 B".to_string(),
            disk_usage_bytes: None,
            disk_usage: None,
        },
    );

    with_docker.metrics.workspaces = vec![docker];

    let stopped = BenchmarkResult::empty(Uuid::now_v7(), "/work/early".to_string());

    repository
        .save(&with_docker, &configuration())
        .await
        .unwrap();

    repository
        .save_run(
            &stopped,
            &configuration(),
            RunOutcome::cancelled(Some("discovery".to_string()), Vec::new()),
        )
        .await
        .unwrap();

    let rows = repository.list_summaries(&query()).await.unwrap();
    let full = rows
        .iter()
        .find(|row| row.id == with_docker.run_id)
        .unwrap();
    let early = rows.iter().find(|row| row.id == stopped.run_id).unwrap();

    assert_eq!(full.build_output_bytes, Some(200));
    assert_eq!(full.workspaces.len(), 1);
    assert_eq!(full.workspaces[0].name, "subscriber");
    assert_eq!(full.workspaces[0].mode, "docker");
    assert_eq!(full.workspaces[0].image_bytes, Some(1000));
    assert_eq!(full.workspaces[0].disk_usage_bytes, None);
    assert_eq!(full.workspaces[0].build_duration_ms, None);

    assert_eq!(early.build_output_bytes, None);
    assert!(early.workspaces.is_empty());

    database.teardown().await;
}

#[tokio::test]
async fn a_saved_target_name_comes_with_the_row() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = run_at("/work/named", Some("bun"));

    repository
        .save(&result, &sample::target_configuration())
        .await
        .unwrap();

    let rows = repository.list_summaries(&query()).await.unwrap();

    assert_eq!(rows[0].target_name.as_deref(), Some("Demo API"));

    database.teardown().await;
}
