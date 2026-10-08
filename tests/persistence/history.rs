use benchmark_api::{
    features::v1::benchmark::{
        metrics::artifact_size::format_size,
        model::{BenchmarkResult, MemoryReading},
    },
    persistence::{
        error::PersistenceError,
        history::{CursorPosition, SummaryQuery, SummaryRow},
        postgres::{mapping::run_created_at, repository::PgBenchmarkRepository},
        repository::BenchmarkRepository,
    },
};
use uuid::Uuid;

use super::{sample, support::setup};

// A UUIDv7 with an exact millisecond timestamp, tail only makes the id unique
fn v7_at(milliseconds: u64, tail: u64) -> Uuid {
    let value = (u128::from(milliseconds) << 80)
        | (0x7u128 << 76)
        | (0b10u128 << 62)
        | u128::from(tail & ((1 << 62) - 1));

    Uuid::from_u128(value)
}

fn query(limit: i64) -> SummaryQuery {
    SummaryQuery {
        limit,
        after: None,
        runtime_code: None,
        target_path: None,
    }
}

// The sample result with its own id, path and runtime
fn run(id: Uuid, target_path: &str, runtime: Option<&str>) -> BenchmarkResult {
    let mut result = sample::result();

    result.run_id = id;
    result.target.path = target_path.to_string();
    result.discovery.runtime = runtime.map(str::to_string);

    result
}

fn ids(rows: &[SummaryRow]) -> Vec<Uuid> {
    rows.iter().map(|row| row.id).collect()
}

// What the POST endpoint really returns: formatted sizes come from the byte counts and the
// build discovery is always null, the sample uses placeholder strings for both
fn as_posted(mut result: BenchmarkResult) -> BenchmarkResult {
    result.discovery.build = None;

    if let Some(memory) = result.metrics.memory.as_mut() {
        for reading in [&mut memory.baseline, &mut memory.peak, &mut memory.delta] {
            *reading = MemoryReading {
                rss_bytes: reading.rss_bytes,
                rss_size: format_size(reading.rss_bytes),
            };
        }
    }

    result
}

#[tokio::test]
async fn an_empty_database_lists_nothing() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    assert!(
        repository
            .list_summaries(&query(21))
            .await
            .unwrap()
            .is_empty()
    );

    database.teardown().await;
}

#[tokio::test]
async fn a_summary_carries_the_stored_metrics() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository.save(&result).await.unwrap();

    let rows = repository.list_summaries(&query(21)).await.unwrap();

    assert_eq!(rows.len(), 1);

    let row = &rows[0];

    assert_eq!(row.id, result.run_id);
    assert_eq!(row.target_path, "/absolute/path/to/project");
    assert_eq!(row.runtime_code.as_deref(), Some("bun"));
    assert_eq!(row.created_at, run_created_at(result.run_id).unwrap());
    assert_eq!(row.build_time_ms, Some(1842));
    assert_eq!(row.startup_time_ms, Some(1500));
    assert_eq!(row.memory_peak_bytes, Some(73_400_320));
    assert_eq!(row.cpu_average_percent, Some(12.4));
    assert_eq!(row.requests_per_second, Some(121.45));
    assert_eq!(row.p95_ms, Some(8.7));

    database.teardown().await;
}

#[tokio::test]
async fn runs_are_listed_newest_first() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let oldest = v7_at(1000, 1);
    let newest = v7_at(3000, 3);
    let middle = v7_at(2000, 2);

    for id in [middle, oldest, newest] {
        repository.save(&run(id, "/a", Some("bun"))).await.unwrap();
    }

    let rows = repository.list_summaries(&query(21)).await.unwrap();

    assert_eq!(ids(&rows), vec![newest, middle, oldest]);

    database.teardown().await;
}

#[tokio::test]
async fn the_limit_is_applied_by_the_database() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    for number in 1..=5 {
        let id = v7_at(number * 1000, number);

        repository.save(&run(id, "/a", Some("bun"))).await.unwrap();
    }

    let rows = repository.list_summaries(&query(2)).await.unwrap();

    assert_eq!(rows.len(), 2);

    database.teardown().await;
}

#[tokio::test]
async fn the_runtime_filter_uses_the_lookup_code() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let bun = v7_at(1000, 1);
    let node = v7_at(2000, 2);
    let none = v7_at(3000, 3);

    repository.save(&run(bun, "/a", Some("bun"))).await.unwrap();
    repository
        .save(&run(node, "/a", Some("node")))
        .await
        .unwrap();
    repository.save(&run(none, "/a", None)).await.unwrap();

    let mut filtered = query(21);

    filtered.runtime_code = Some("bun".to_string());

    let rows = repository.list_summaries(&filtered).await.unwrap();

    assert_eq!(ids(&rows), vec![bun]);

    database.teardown().await;
}

#[tokio::test]
async fn an_unknown_runtime_code_is_reported() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut filtered = query(21);

    filtered.runtime_code = Some("cobol".to_string());

    let error = repository.list_summaries(&filtered).await.unwrap_err();

    assert_eq!(
        error,
        PersistenceError::UnknownLookup {
            table: "runtimes",
            code: "cobol".to_string()
        }
    );

    database.teardown().await;
}

#[tokio::test]
async fn the_target_path_filter_is_an_exact_match() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let exact = v7_at(1000, 1);

    repository
        .save(&run(exact, "/projects/api", Some("bun")))
        .await
        .unwrap();
    repository
        .save(&run(v7_at(2000, 2), "/projects/api/nested", Some("bun")))
        .await
        .unwrap();
    repository
        .save(&run(v7_at(3000, 3), "/projects", Some("bun")))
        .await
        .unwrap();

    let mut filtered = query(21);

    filtered.target_path = Some("/projects/api".to_string());

    let rows = repository.list_summaries(&filtered).await.unwrap();

    assert_eq!(ids(&rows), vec![exact]);

    database.teardown().await;
}

#[tokio::test]
async fn runtime_and_target_path_filters_combine() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let both = v7_at(1000, 1);

    repository
        .save(&run(both, "/a", Some("bun")))
        .await
        .unwrap();
    repository
        .save(&run(v7_at(2000, 2), "/a", Some("node")))
        .await
        .unwrap();
    repository
        .save(&run(v7_at(3000, 3), "/b", Some("bun")))
        .await
        .unwrap();

    let mut filtered = query(21);

    filtered.runtime_code = Some("bun".to_string());
    filtered.target_path = Some("/a".to_string());

    let rows = repository.list_summaries(&filtered).await.unwrap();

    assert_eq!(ids(&rows), vec![both]);

    database.teardown().await;
}

#[tokio::test]
async fn the_cursor_pages_through_every_run_once() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut expected = Vec::new();

    for number in 1..=5 {
        let id = v7_at(number * 1000, number);

        repository.save(&run(id, "/a", Some("bun"))).await.unwrap();

        expected.insert(0, id);
    }

    let mut seen = Vec::new();
    let mut paged = query(2);

    loop {
        let rows = repository.list_summaries(&paged).await.unwrap();

        if rows.is_empty() {
            break;
        }

        seen.extend(ids(&rows));

        let last = rows.last().unwrap();

        paged.after = Some(CursorPosition {
            created_at: last.created_at,
            id: last.id,
        });
    }

    assert_eq!(seen, expected);

    database.teardown().await;
}

#[tokio::test]
async fn filters_apply_before_the_cursor() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let bun_old = v7_at(1000, 1);
    let bun_new = v7_at(4000, 4);

    repository
        .save(&run(bun_old, "/a", Some("bun")))
        .await
        .unwrap();
    repository
        .save(&run(v7_at(2000, 2), "/a", Some("node")))
        .await
        .unwrap();
    repository
        .save(&run(v7_at(3000, 3), "/a", Some("node")))
        .await
        .unwrap();
    repository
        .save(&run(bun_new, "/a", Some("bun")))
        .await
        .unwrap();

    let mut paged = query(1);

    paged.runtime_code = Some("bun".to_string());

    let first = repository.list_summaries(&paged).await.unwrap();

    assert_eq!(ids(&first), vec![bun_new]);

    paged.after = Some(CursorPosition {
        created_at: first[0].created_at,
        id: first[0].id,
    });

    let second = repository.list_summaries(&paged).await.unwrap();

    assert_eq!(ids(&second), vec![bun_old]);

    database.teardown().await;
}

#[tokio::test]
async fn runs_with_the_same_created_at_are_ordered_by_id() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let low = v7_at(5000, 1);
    let middle = v7_at(5000, 2);
    let high = v7_at(5000, 3);

    for id in [middle, low, high] {
        repository.save(&run(id, "/a", Some("bun"))).await.unwrap();
    }

    let all = repository.list_summaries(&query(21)).await.unwrap();

    assert_eq!(ids(&all), vec![high, middle, low]);

    // Paging one row at a time never skips or repeats a row inside the tie
    let mut seen = Vec::new();
    let mut paged = query(1);

    loop {
        let rows = repository.list_summaries(&paged).await.unwrap();

        if rows.is_empty() {
            break;
        }

        seen.extend(ids(&rows));

        paged.after = Some(CursorPosition {
            created_at: rows[0].created_at,
            id: rows[0].id,
        });
    }

    assert_eq!(seen, vec![high, middle, low]);

    database.teardown().await;
}

#[tokio::test]
async fn a_run_without_runtime_or_metrics_has_null_summary_values() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::skipped_result();

    repository.save(&result).await.unwrap();

    let rows = repository.list_summaries(&query(21)).await.unwrap();

    assert_eq!(rows.len(), 1);

    let row = &rows[0];

    assert_eq!(row.runtime_code, None);
    assert_eq!(row.build_time_ms, None);
    assert_eq!(row.startup_time_ms, None);
    assert_eq!(row.memory_peak_bytes, None);
    assert_eq!(row.cpu_average_percent, None);
    assert_eq!(row.requests_per_second, None);
    assert_eq!(row.p95_ms, None);

    database.teardown().await;
}

#[tokio::test]
async fn a_stored_run_is_rebuilt_exactly_like_the_post_result() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let posted = as_posted(sample::result());

    repository.save(&posted).await.unwrap();

    let fetched = repository
        .get_by_run_id(posted.run_id)
        .await
        .unwrap()
        .expect("the stored run must be found");

    assert_eq!(
        serde_json::to_value(&fetched).unwrap(),
        serde_json::to_value(&posted).unwrap()
    );

    database.teardown().await;
}

#[tokio::test]
async fn a_run_with_nullable_runtime_and_skipped_metrics_is_rebuilt_exactly() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let posted = as_posted(sample::skipped_result());

    repository.save(&posted).await.unwrap();

    let fetched = repository
        .get_by_run_id(posted.run_id)
        .await
        .unwrap()
        .expect("the stored run must be found");

    assert_eq!(fetched.discovery.runtime, None);
    assert!(fetched.metrics.memory.is_none());
    assert!(fetched.metrics.cpu.is_none());
    assert!(fetched.metrics.http_latency.is_none());
    assert_eq!(
        serde_json::to_value(&fetched).unwrap(),
        serde_json::to_value(&posted).unwrap()
    );

    database.teardown().await;
}

#[tokio::test]
async fn a_run_that_does_not_exist_is_none() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let fetched = repository.get_by_run_id(Uuid::now_v7()).await.unwrap();

    assert!(fetched.is_none());

    database.teardown().await;
}
