use benchmark_api::{
    features::v1::benchmark::model::BenchmarkResult,
    persistence::{
        history::{SummaryQuery, TrendRow},
        postgres::repository::PgBenchmarkRepository,
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

// The id of run number `number`, one second after the previous number
fn id(number: u64) -> Uuid {
    v7_at(number * 1000, number)
}

// The complete sample result with its own id
fn run(number: u64) -> BenchmarkResult {
    let mut result = sample::result();

    result.run_id = id(number);

    result
}

// The trend row of one stored run, the test fails when the run is not in the rows
fn row_of(rows: &[TrendRow], number: u64) -> &TrendRow {
    rows.iter()
        .find(|row| row.id == id(number))
        .expect("the run must be in the trend")
}

fn ids(rows: &[TrendRow]) -> Vec<Uuid> {
    rows.iter().map(|row| row.id).collect()
}

#[tokio::test]
async fn an_empty_database_has_no_runs_and_no_trends() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    assert_eq!(repository.count_runs().await.unwrap(), 0);
    assert!(repository.list_trends(30).await.unwrap().is_empty());

    database.teardown().await;
}

#[tokio::test]
async fn every_stored_run_is_counted() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    repository.save(&run(1)).await.unwrap();
    repository.save(&run(2)).await.unwrap();

    let mut skipped = sample::skipped_result();

    skipped.run_id = id(3);

    repository.save(&skipped).await.unwrap();

    assert_eq!(repository.count_runs().await.unwrap(), 3);

    database.teardown().await;
}

#[tokio::test]
async fn trends_are_newest_first_and_limited() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    for number in 1..=5 {
        repository.save(&run(number)).await.unwrap();
    }

    let rows = repository.list_trends(3).await.unwrap();

    assert_eq!(ids(&rows), vec![id(5), id(4), id(3)]);

    database.teardown().await;
}

#[tokio::test]
async fn runs_with_the_same_created_at_are_ordered_by_id() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    for tail in 1..=3 {
        let mut result = sample::result();

        result.run_id = v7_at(5000, tail);

        repository.save(&result).await.unwrap();
    }

    let rows = repository.list_trends(30).await.unwrap();

    assert_eq!(
        ids(&rows),
        vec![v7_at(5000, 3), v7_at(5000, 2), v7_at(5000, 1)]
    );

    database.teardown().await;
}

#[tokio::test]
async fn a_complete_successful_run_keeps_every_stored_value() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    repository.save(&run(1)).await.unwrap();

    let rows = repository.list_trends(30).await.unwrap();

    assert_eq!(rows.len(), 1);

    let row = &rows[0];

    assert_eq!(row.id, id(1));
    assert_eq!(row.build_time_ms, Some(1842));
    assert_eq!(row.startup_time_ms, Some(1500));
    assert_eq!(row.memory_peak_bytes, Some(73_400_320));
    assert_eq!(row.cpu_average_percent, Some(12.4));
    assert_eq!(row.requests_per_second, Some(121.45));
    assert_eq!(row.p95_ms, Some(8.7));

    database.teardown().await;
}

#[tokio::test]
async fn a_build_that_did_not_succeed_has_no_trend_duration() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut failed = run(1);

    failed.metrics.build_time.status = "failed".to_string();
    failed.metrics.build_time.exit_code = Some(1);

    let mut timed_out = run(2);

    timed_out.metrics.build_time.status = "timeout".to_string();
    timed_out.metrics.build_time.exit_code = None;

    repository.save(&failed).await.unwrap();
    repository.save(&timed_out).await.unwrap();
    repository.save(&run(3)).await.unwrap();

    let rows = repository.list_trends(30).await.unwrap();

    assert_eq!(row_of(&rows, 1).build_time_ms, None);
    assert_eq!(row_of(&rows, 2).build_time_ms, None);
    assert_eq!(row_of(&rows, 3).build_time_ms, Some(1842));

    // Only the build is affected, the startup of the same runs still counts
    assert_eq!(row_of(&rows, 1).startup_time_ms, Some(1500));

    database.teardown().await;
}

#[tokio::test]
async fn a_startup_that_did_not_succeed_has_no_trend_duration() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    for (number, status) in [(1, "failed"), (2, "timeout"), (3, "unhealthy")] {
        let mut result = run(number);

        result.metrics.startup_time.status = status.to_string();
        result.metrics.startup_time.ready_at = None;
        result.metrics.startup_time.exit_code = None;

        repository.save(&result).await.unwrap();
    }

    repository.save(&run(4)).await.unwrap();

    let rows = repository.list_trends(30).await.unwrap();

    for number in 1..=3 {
        assert_eq!(row_of(&rows, number).startup_time_ms, None, "run {number}");
    }

    assert_eq!(row_of(&rows, 4).startup_time_ms, Some(1500));

    // Only the startup is affected, the build of the same runs still counts
    assert_eq!(row_of(&rows, 1).build_time_ms, Some(1842));

    database.teardown().await;
}

#[tokio::test]
async fn a_partial_workload_has_no_trend_http_metrics() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut result = run(1);

    result.metrics.http_latency.as_mut().unwrap().status = "target_exited".to_string();

    repository.save(&result).await.unwrap();

    let rows = repository.list_trends(30).await.unwrap();
    let row = row_of(&rows, 1);

    assert_eq!(row.requests_per_second, None);
    assert_eq!(row.p95_ms, None);

    // Memory and CPU have no status and are kept
    assert_eq!(row.memory_peak_bytes, Some(73_400_320));
    assert_eq!(row.cpu_average_percent, Some(12.4));

    database.teardown().await;
}

#[tokio::test]
async fn zero_observed_requests_are_null_and_not_a_zero_throughput() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut result = run(1);
    let latency = result.metrics.http_latency.as_mut().unwrap();

    latency.total_requests = 0;
    latency.successful_requests = 0;
    latency.failed_requests = 0;
    latency.requests_per_second = 0.0;
    latency.min_ms = None;
    latency.mean_ms = None;
    latency.p50_ms = None;
    latency.p95_ms = None;
    latency.p99_ms = None;
    latency.max_ms = None;

    repository.save(&result).await.unwrap();

    let rows = repository.list_trends(30).await.unwrap();
    let row = row_of(&rows, 1);

    assert_eq!(row.requests_per_second, None);
    assert_eq!(row.p95_ms, None);

    database.teardown().await;
}

#[tokio::test]
async fn a_run_without_a_workload_has_null_memory_cpu_and_http_metrics() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut result = run(1);

    result.metrics.memory = None;
    result.metrics.cpu = None;
    result.metrics.http_latency = None;

    repository.save(&result).await.unwrap();

    let rows = repository.list_trends(30).await.unwrap();
    let row = row_of(&rows, 1);

    assert_eq!(row.build_time_ms, Some(1842));
    assert_eq!(row.startup_time_ms, Some(1500));
    assert_eq!(row.memory_peak_bytes, None);
    assert_eq!(row.cpu_average_percent, None);
    assert_eq!(row.requests_per_second, None);
    assert_eq!(row.p95_ms, None);

    database.teardown().await;
}

#[tokio::test]
async fn a_run_where_every_step_was_skipped_has_six_null_metrics() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut result = sample::skipped_result();

    result.run_id = id(1);

    repository.save(&result).await.unwrap();

    let rows = repository.list_trends(30).await.unwrap();

    assert_eq!(
        rows,
        vec![TrendRow {
            id: id(1),
            created_at: rows[0].created_at,
            build_time_ms: None,
            startup_time_ms: None,
            memory_peak_bytes: None,
            cpu_average_percent: None,
            requests_per_second: None,
            p95_ms: None,
        }]
    );

    database.teardown().await;
}

#[tokio::test]
async fn a_measured_zero_stays_zero() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut result = run(1);

    result.metrics.build_time.duration_ms = Some(0);

    let memory = result.metrics.memory.as_mut().unwrap();

    memory.baseline.rss_bytes = 0;
    memory.peak.rss_bytes = 0;
    memory.delta.rss_bytes = 0;

    result.metrics.cpu.as_mut().unwrap().average_percent = 0.0;
    result
        .metrics
        .http_latency
        .as_mut()
        .unwrap()
        .requests_per_second = 0.0;

    repository.save(&result).await.unwrap();

    let rows = repository.list_trends(30).await.unwrap();
    let row = row_of(&rows, 1);

    assert_eq!(row.build_time_ms, Some(0));
    assert_eq!(row.memory_peak_bytes, Some(0));
    assert_eq!(row.cpu_average_percent, Some(0.0));
    assert_eq!(row.requests_per_second, Some(0.0));

    database.teardown().await;
}

#[tokio::test]
async fn the_history_summary_keeps_the_raw_values_the_trend_nulls() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut result = run(1);

    result.metrics.build_time.status = "failed".to_string();
    result.metrics.build_time.exit_code = Some(1);
    result.metrics.http_latency.as_mut().unwrap().status = "target_exited".to_string();

    repository.save(&result).await.unwrap();

    let query = SummaryQuery {
        limit: 1,
        after: None,
        runtime_code: None,
        target_path: None,
    };

    let summaries = repository.list_summaries(&query).await.unwrap();
    let trends = repository.list_trends(30).await.unwrap();

    // The frozen history summary is unchanged and still reads the stored values
    assert_eq!(summaries[0].build_time_ms, Some(1842));
    assert_eq!(summaries[0].requests_per_second, Some(121.45));
    assert_eq!(summaries[0].p95_ms, Some(8.7));

    // The trend of the same run follows the comparison rules
    assert_eq!(trends[0].build_time_ms, None);
    assert_eq!(trends[0].requests_per_second, None);
    assert_eq!(trends[0].p95_ms, None);

    database.teardown().await;
}
