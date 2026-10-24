use super::sample::configuration;
use benchmark_api::{
    features::v1::comparison::{error::ComparisonError, query::RunIds, service},
    persistence::{postgres::repository::PgBenchmarkRepository, repository::BenchmarkRepository},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{sample, support::setup};

fn ids(runs: &[benchmark_api::persistence::stored_run::StoredRun]) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = runs.iter().map(|run| run.result.run_id).collect();

    ids.sort();

    ids
}

#[tokio::test]
async fn only_the_requested_runs_are_read() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let mut stored = Vec::new();

    for _ in 0..3 {
        let result = sample::result();

        stored.push(result.run_id);
        repository.save(&result, &configuration()).await.unwrap();
    }

    let requested = [stored[0], stored[2]];

    let runs = repository.get_runs(&requested).await.unwrap();

    let mut expected = requested.to_vec();

    expected.sort();

    assert_eq!(ids(&runs), expected);

    database.teardown().await;
}

#[tokio::test]
async fn an_absent_id_is_missing_from_the_result() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository.save(&result, &configuration()).await.unwrap();

    let runs = repository
        .get_runs(&[result.run_id, Uuid::now_v7()])
        .await
        .unwrap();

    assert_eq!(ids(&runs), vec![result.run_id]);

    assert!(repository.get_runs(&[]).await.unwrap().is_empty());

    database.teardown().await;
}

#[tokio::test]
async fn a_run_read_in_bulk_equals_the_same_run_read_alone() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let full = sample::result();
    let skipped = sample::skipped_result();

    repository.save(&full, &configuration()).await.unwrap();
    repository.save(&skipped, &configuration()).await.unwrap();

    let runs = repository
        .get_runs(&[full.run_id, skipped.run_id])
        .await
        .unwrap();

    assert_eq!(runs.len(), 2);

    for run in &runs {
        let alone = repository
            .get_by_run_id(run.result.run_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            serde_json::to_value(&run.result).unwrap(),
            serde_json::to_value(&alone).unwrap()
        );
    }

    database.teardown().await;
}

#[tokio::test]
async fn two_stored_runs_are_compared_from_sql() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let baseline = sample::result();

    // The comparison run: half the build time, double CPU, half the peak memory
    let mut comparison = sample::result();

    comparison.metrics.build_time.duration_ms = Some(921);
    comparison.metrics.cpu.as_mut().unwrap().average_percent = 24.8;
    comparison.metrics.memory.as_mut().unwrap().peak.rss_bytes = 36_700_160;
    comparison.metrics.artifact_size.build.bytes = 400;

    repository.save(&baseline, &configuration()).await.unwrap();
    repository
        .save(&comparison, &configuration())
        .await
        .unwrap();

    let ids = RunIds {
        baseline: baseline.run_id,
        comparison: comparison.run_id,
    };

    let response = service::compare(&repository, ids).await.unwrap();
    let value: Value = serde_json::to_value(&response).unwrap();
    let metrics = &value["metrics"];

    assert_eq!(
        value["baseline"]["run_id"],
        json!(baseline.run_id.to_string())
    );
    assert_eq!(
        value["comparison"]["run_id"],
        json!(comparison.run_id.to_string())
    );

    let build = &metrics["build_time"]["duration_ms"];

    assert_eq!(build["baseline_value"], json!(1842));
    assert_eq!(build["comparison_value"], json!(921));
    assert_eq!(build["absolute_difference"], json!(-921));
    assert_eq!(build["percentage_change"], json!(-50.0));

    let cpu = &metrics["cpu"]["average_percent"];

    assert_eq!(cpu["baseline_value"], json!(12.4));
    assert_eq!(cpu["comparison_value"], json!(24.8));
    assert_eq!(cpu["percentage_change"], json!(100.0));

    let peak = &metrics["memory"]["peak_rss_bytes"];

    assert_eq!(peak["absolute_difference"], json!(-36_700_160));
    assert_eq!(peak["percentage_change"], json!(-50.0));

    let artifact = &metrics["artifact_size"]["build"]["bytes"];

    assert_eq!(artifact["baseline_value"], json!(200));
    assert_eq!(artifact["comparison_value"], json!(400));
    assert_eq!(artifact["percentage_change"], json!(100.0));

    assert_eq!(
        metrics["http_latency"]["requests_per_second"]["direction"],
        json!("higher_is_better")
    );

    database.teardown().await;
}

#[tokio::test]
async fn a_skipped_stored_run_compares_with_null_values() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    let baseline = sample::result();
    let skipped = sample::skipped_result();

    repository.save(&baseline, &configuration()).await.unwrap();
    repository.save(&skipped, &configuration()).await.unwrap();

    let ids = RunIds {
        baseline: baseline.run_id,
        comparison: skipped.run_id,
    };

    let response = service::compare(&repository, ids).await.unwrap();
    let value = serde_json::to_value(&response).unwrap();
    let metrics = &value["metrics"];

    assert_eq!(
        metrics["build_time"]["comparison_status"],
        json!("no_build_command")
    );
    assert_eq!(
        metrics["build_time"]["duration_ms"]["comparison_value"],
        json!(null)
    );
    assert_eq!(
        metrics["memory"]["peak_rss_bytes"]["comparison_value"],
        json!(null)
    );
    assert_eq!(
        metrics["http_latency"]["p95_ms"]["absolute_difference"],
        json!(null)
    );
    assert_eq!(value["comparison"]["runtime"], json!(null));

    database.teardown().await;
}

#[tokio::test]
async fn a_run_missing_from_the_database_is_not_found() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let stored = sample::result();

    repository.save(&stored, &configuration()).await.unwrap();

    let missing = Uuid::now_v7();

    let error = service::compare(
        &repository,
        RunIds {
            baseline: stored.run_id,
            comparison: missing,
        },
    )
    .await
    .err()
    .expect("a missing run must fail");

    assert!(
        matches!(&error, ComparisonError::NotFound(message) if message.contains(&missing.to_string()))
    );

    database.teardown().await;
}
