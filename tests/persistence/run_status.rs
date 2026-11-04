use benchmark_api::{
    features::v1::{benchmark::model::BenchmarkResult, comparison::compare::compare},
    persistence::{
        history::SummaryQuery,
        outcome::{RunOutcome, StageRecord, StoredStatus},
        postgres::{mapping::run_created_at, repository::PgBenchmarkRepository},
        repository::BenchmarkRepository,
    },
};
use uuid::Uuid;

use super::{
    sample::{self, configuration},
    support::{count_for_run, setup},
};

// A run that stopped: only what finished is kept
fn stopped_after(steps: &[&str]) -> BenchmarkResult {
    let full = sample::result();
    let mut result = BenchmarkResult::empty(Uuid::now_v7(), full.target.path.clone());

    if steps.contains(&"discovery") {
        result.discovery = full.discovery.clone();
    }

    if steps.contains(&"artifact_size") {
        result.metrics.artifact_size = full.metrics.artifact_size.clone();
    }

    if steps.contains(&"build") {
        result.metrics.build_time = full.metrics.build_time.clone();
    }

    if steps.contains(&"startup") {
        result.metrics.startup_time = full.metrics.startup_time.clone();
    }

    if steps.contains(&"workload") {
        result.metrics.memory = full.metrics.memory.clone();
        result.metrics.cpu = full.metrics.cpu.clone();
        result.metrics.http_latency = full.metrics.http_latency.clone();
    }

    result
}

fn stage(name: &str, status: &str, duration_ms: u64) -> StageRecord {
    StageRecord {
        stage: name.to_string(),
        status: status.to_string(),
        duration_ms: Some(duration_ms),
    }
}

// One status code at a time: only these step statuses make a run failed
#[test]
fn every_status_code_gives_the_run_the_status_the_rule_says() {
    let cases = [
        ("success", StoredStatus::Done),
        ("completed", StoredStatus::Done),
        ("no_build_command", StoredStatus::Done),
        ("no_start_command", StoredStatus::Done),
        ("no_port", StoredStatus::Done),
        ("failed", StoredStatus::Failed),
        ("timeout", StoredStatus::Failed),
        ("unhealthy", StoredStatus::Failed),
        ("target_exited", StoredStatus::Failed),
    ];

    for (code, expected) in cases {
        let mut build = sample::result();
        build.metrics.build_time.as_mut().unwrap().status = code.to_string();

        let mut startup = sample::result();
        startup.metrics.startup_time.as_mut().unwrap().status = code.to_string();

        let mut load = sample::result();
        load.metrics.http_latency.as_mut().unwrap().status = code.to_string();

        for result in [build, startup, load] {
            assert_eq!(
                RunOutcome::from_result(&result).status,
                expected,
                "status code {code}"
            );
        }
    }
}

#[test]
fn a_run_with_no_workload_at_all_is_done() {
    let result = sample::skipped_result();

    let outcome = RunOutcome::from_result(&result);

    assert_eq!(outcome.status, StoredStatus::Done);
    assert_eq!(outcome.stopped_stage, None);
    assert_eq!(outcome.failure_message, None);
}

#[test]
fn a_failed_step_names_its_stage_and_the_first_failure_wins() {
    let mut result = sample::result();
    result.metrics.startup_time.as_mut().unwrap().status = "timeout".to_string();
    result.metrics.http_latency.as_mut().unwrap().status = "target_exited".to_string();

    let outcome = RunOutcome::from_result(&result);

    assert_eq!(outcome.status, StoredStatus::Failed);
    assert_eq!(outcome.stopped_stage.as_deref(), Some("startup"));
    assert_eq!(
        outcome.failure_message.as_deref(),
        Some("The startup timed out.")
    );
}

#[tokio::test]
async fn a_failed_workspace_mode_does_not_fail_the_run() {
    use benchmark_api::features::v1::benchmark::workspace::result::WorkspaceResult;

    let mut result = sample::result();
    let mut workspace = WorkspaceResult::empty("subscriber", "docker", Vec::new());
    workspace.status = "failed".to_string();
    result.metrics.workspaces.push(workspace);

    assert_eq!(RunOutcome::from_result(&result).status, StoredStatus::Done);
}

#[tokio::test]
async fn a_run_stopped_at_each_stage_saves_lists_opens_and_compares() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    // (stopped stage, steps that finished before it)
    let cases: [(&str, &[&str]); 5] = [
        ("discovery", &[]),
        ("build", &["discovery", "artifact_size"]),
        ("startup", &["discovery", "artifact_size", "build"]),
        (
            "workload",
            &["discovery", "artifact_size", "build", "startup"],
        ),
        (
            "workspaces",
            &["discovery", "artifact_size", "build", "startup", "workload"],
        ),
    ];

    let complete = sample::result();
    repository.save(&complete, &configuration()).await.unwrap();

    for (stopped, finished) in cases {
        let result = stopped_after(finished);
        let run_id = result.run_id;

        let outcome = RunOutcome::cancelled(
            Some(stopped.to_string()),
            vec![stage(stopped, "cancelled", 40)],
        );

        repository
            .save_run(&result, &configuration(), outcome)
            .await
            .unwrap();

        // Opened: parts that did not finish are null, never an error and never a zero
        let runs = repository.get_runs(&[run_id]).await.unwrap();
        let stored = &runs[0];

        assert_eq!(stored.status, StoredStatus::Cancelled, "{stopped}");
        assert_eq!(stored.stopped_stage.as_deref(), Some(stopped));
        assert_eq!(stored.failure_message, None);
        assert_eq!(
            stored.result.metrics.build_time.is_some(),
            finished.contains(&"build")
        );
        assert_eq!(
            stored.result.metrics.startup_time.is_some(),
            finished.contains(&"startup")
        );
        assert_eq!(
            stored.result.metrics.artifact_size.is_some(),
            finished.contains(&"artifact_size")
        );
        assert_eq!(
            stored.result.metrics.http_latency.is_some(),
            finished.contains(&"workload")
        );

        // The stage rows: the one that stopped, then the saving row
        let stage_names: Vec<&str> = stored.stages.iter().map(|s| s.stage.as_str()).collect();
        assert_eq!(stage_names, vec![stopped, "saving"]);
        assert_eq!(stored.stages[0].status, "cancelled");

        // Compared with a complete run, either way round
        let complete_stored = repository.get_runs(&[complete.run_id]).await.unwrap();
        let _ = compare(stored, &complete_stored[0]);
        let _ = compare(&complete_stored[0], stored);
    }

    // Listed: every run is there, newest first, nothing failed on the way
    let listed = repository
        .list_summaries(&SummaryQuery {
            limit: 50,
            after: None,
            runtime_code: None,
            target_path: None,
            status: None,
            search: None,
        })
        .await
        .unwrap();

    assert_eq!(listed.len(), 6);

    database.teardown().await;
}

#[tokio::test]
async fn a_failed_run_keeps_its_message_and_stage_rows() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = stopped_after(&[]);
    let run_id = result.run_id;

    let outcome = RunOutcome::failed(
        Some("discovery".to_string()),
        "Two manifests were found.".to_string(),
        vec![stage("discovery", "failed", 3)],
    );

    repository
        .save_run(&result, &configuration(), outcome)
        .await
        .unwrap();

    let stored = repository
        .get_runs(&[run_id])
        .await
        .unwrap()
        .remove(0)
        .result;

    assert!(stored.metrics.build_time.is_none());
    assert!(stored.metrics.artifact_size.is_none());
    assert_eq!(
        count_for_run(&database.pool, "benchmark_run_stages", run_id).await,
        2
    );

    let status: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT status, stopped_stage, failure_message FROM benchmark_runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();

    assert_eq!(status.0, "failed");
    assert_eq!(status.1.as_deref(), Some("discovery"));
    assert_eq!(status.2.as_deref(), Some("Two manifests were found."));

    let _ = run_created_at(run_id).unwrap();

    database.teardown().await;
}

#[tokio::test]
async fn a_path_with_trailing_slashes_is_stored_without_them() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let mut result = sample::result();
    result.target.path = "/absolute/path/to/project///".to_string();

    repository.save(&result, &configuration()).await.unwrap();

    let stored: String = sqlx::query_scalar("SELECT target_path FROM benchmark_runs WHERE id = $1")
        .bind(result.run_id)
        .fetch_one(&database.pool)
        .await
        .unwrap();

    assert_eq!(stored, "/absolute/path/to/project");

    database.teardown().await;
}
