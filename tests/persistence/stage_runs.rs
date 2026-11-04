use benchmark_api::persistence::{
    outcome::{RunOutcome, StageRecord},
    postgres::repository::PgBenchmarkRepository,
    repository::BenchmarkRepository,
};
use uuid::Uuid;

use super::{
    sample::{self, configuration},
    support::setup,
};

fn id(number: u64) -> Uuid {
    let value =
        (u128::from(number * 1000) << 80) | (0x7u128 << 76) | (0b10u128 << 62) | u128::from(number);

    Uuid::from_u128(value)
}

fn stage(name: &str, duration_ms: Option<u64>) -> StageRecord {
    StageRecord {
        stage: name.to_string(),
        status: "success".to_string(),
        duration_ms,
    }
}

// Stores a done run with its stage rows, a failed build makes it failed
async fn save(
    repository: &PgBenchmarkRepository,
    number: u64,
    path: &str,
    failed: bool,
    stages: Vec<StageRecord>,
) -> Uuid {
    let mut result = sample::result();

    result.run_id = id(number);
    result.target.path = path.to_string();

    if failed {
        result.metrics.build_time.as_mut().unwrap().status = "failed".to_string();
    }

    let outcome = RunOutcome::from_result(&result).with_stages(stages);

    repository
        .save_run(&result, &configuration(), outcome)
        .await
        .unwrap();

    result.run_id
}

#[tokio::test]
async fn only_the_newest_done_runs_of_the_target_are_read_with_their_stages_in_order() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let api = "/projects/api";

    save(&repository, 1, api, false, vec![stage("build", Some(9))]).await;
    save(
        &repository,
        2,
        api,
        false,
        vec![stage("build", Some(10)), stage("startup", None)],
    )
    .await;
    // A run with no stage of its own still has the saving stage the database adds
    save(&repository, 3, api, false, vec![]).await;
    save(&repository, 4, api, true, vec![stage("build", Some(99))]).await;
    save(
        &repository,
        5,
        "/projects/web",
        false,
        vec![stage("build", Some(77))],
    )
    .await;
    let deleted = save(&repository, 6, api, false, vec![stage("build", Some(55))]).await;

    repository.delete_run(deleted).await.unwrap();

    let runs = repository.recent_stage_runs(api, 3).await.unwrap();

    assert_eq!(
        runs.iter().map(|run| run.run_id).collect::<Vec<_>>(),
        vec![id(3), id(2), id(1)]
    );
    assert_eq!(
        runs[0]
            .stages
            .iter()
            .map(|stage| stage.stage.as_str())
            .collect::<Vec<_>>(),
        vec!["saving"]
    );
    assert_eq!(
        runs[1]
            .stages
            .iter()
            .take(2)
            .map(|stage| (stage.stage.as_str(), stage.position, stage.duration_ms))
            .collect::<Vec<_>>(),
        vec![("build", 0, Some(10)), ("startup", 1, None)]
    );
    assert_eq!(runs[1].stages[2].stage, "saving");
    assert_eq!(runs[2].stages[0].duration_ms, Some(9));

    assert!(
        repository
            .recent_stage_runs("/projects/none", 3)
            .await
            .unwrap()
            .is_empty()
    );

    database.teardown().await;
}

#[tokio::test]
async fn the_database_answers_a_ping() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    repository.ping().await.unwrap();

    database.teardown().await;
}
