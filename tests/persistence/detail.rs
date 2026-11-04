use benchmark_api::{
    features::v1::benchmark::{history::service, model::BenchmarkResult},
    persistence::{
        outcome::{RunOutcome, StageRecord},
        postgres::repository::PgBenchmarkRepository,
        repository::BenchmarkRepository,
    },
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    sample::{self, configuration, target_configuration},
    support::setup,
};

fn stage(name: &str, status: &str, duration_ms: u64) -> StageRecord {
    StageRecord {
        stage: name.to_string(),
        status: status.to_string(),
        duration_ms: Some(duration_ms),
    }
}

async fn detail(repository: &PgBenchmarkRepository, run_id: Uuid) -> Value {
    let detail = service::get(repository, run_id).await.unwrap();

    serde_json::to_value(detail).unwrap()
}

#[tokio::test]
async fn a_complete_run_has_its_status_name_and_stage_times_next_to_the_result() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository
        .save_run(
            &result,
            &target_configuration(),
            RunOutcome::from_result(&result).with_stages(vec![
                stage("discovery", "success", 5),
                stage("build", "success", 1842),
            ]),
        )
        .await
        .unwrap();

    let body = detail(&repository, result.run_id).await;

    assert_eq!(body["run_id"], json!(result.run_id.to_string()));
    assert_eq!(body["status"], json!("done"));
    assert_eq!(body["stopped_stage"], json!(null));
    assert_eq!(body["failure_message"], json!(null));
    assert_eq!(body["display_name"], json!("Demo API"));
    assert_eq!(body["metrics"]["build_time"]["duration_ms"], json!(1842));
    assert_eq!(body["metrics"]["http_latency"]["p95_ms"], json!(8.7));

    // In run order, the saving row is added by the repository
    let names: Vec<&str> = body["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["stage"].as_str().unwrap())
        .collect();

    assert_eq!(names, vec!["discovery", "build", "saving"]);
    assert_eq!(
        body["stages"][1],
        json!({ "stage": "build", "status": "success", "duration_ms": 1842 })
    );

    database.teardown().await;
}

#[tokio::test]
async fn a_custom_path_is_named_after_its_last_folder() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = sample::result();

    repository.save(&result, &configuration()).await.unwrap();

    let body = detail(&repository, result.run_id).await;

    assert_eq!(body["display_name"], json!("project"));
    assert_eq!(body["stages"].as_array().unwrap().len(), 1);
    assert_eq!(body["stages"][0]["stage"], json!("saving"));

    database.teardown().await;
}

#[tokio::test]
async fn a_failed_run_with_no_parts_shows_its_message_and_null_parts() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let result = BenchmarkResult::empty(Uuid::now_v7(), "/work/early/".to_string());

    repository
        .save_run(
            &result,
            &configuration(),
            RunOutcome::failed(
                Some("discovery".to_string()),
                "No runtime was found.".to_string(),
                vec![stage("discovery", "failed", 3)],
            ),
        )
        .await
        .unwrap();

    let body = detail(&repository, result.run_id).await;

    assert_eq!(body["status"], json!("failed"));
    assert_eq!(body["stopped_stage"], json!("discovery"));
    assert_eq!(body["failure_message"], json!("No runtime was found."));
    assert_eq!(body["display_name"], json!("early"));
    assert_eq!(body["target"], json!({ "path": "/work/early" }));
    assert_eq!(body["metrics"]["artifact_size"], json!(null));
    assert_eq!(body["metrics"]["build_time"], json!(null));
    assert_eq!(body["metrics"]["startup_time"], json!(null));
    assert_eq!(body["metrics"]["memory"], json!(null));
    assert_eq!(body["metrics"]["http_latency"], json!(null));
    // A run with no workspace has no workspaces key, as for every run before
    assert!(body["metrics"].get("workspaces").is_none());
    assert_eq!(body["stages"][0]["status"], json!("failed"));

    database.teardown().await;
}

#[tokio::test]
async fn a_cancelled_run_keeps_its_finished_parts_and_the_stage_it_stopped_in() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let full = sample::result();
    let mut result = BenchmarkResult::empty(Uuid::now_v7(), full.target.path.clone());

    result.metrics.artifact_size = full.metrics.artifact_size.clone();
    result.metrics.build_time = full.metrics.build_time.clone();

    repository
        .save_run(
            &result,
            &configuration(),
            RunOutcome::cancelled(
                Some("startup".to_string()),
                vec![
                    stage("build", "success", 1842),
                    stage("startup", "cancelled", 700),
                ],
            ),
        )
        .await
        .unwrap();

    let body = detail(&repository, result.run_id).await;

    assert_eq!(body["status"], json!("cancelled"));
    assert_eq!(body["stopped_stage"], json!("startup"));
    assert_eq!(body["failure_message"], json!(null));
    assert_eq!(body["metrics"]["build_time"]["status"], json!("success"));
    assert_eq!(body["metrics"]["startup_time"], json!(null));
    assert_eq!(body["stages"][1]["status"], json!("cancelled"));

    database.teardown().await;
}

#[tokio::test]
async fn a_run_that_does_not_exist_is_not_found() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    assert!(service::get(&repository, Uuid::now_v7()).await.is_err());

    database.teardown().await;
}
