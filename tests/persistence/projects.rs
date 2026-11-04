use benchmark_api::persistence::{
    history::TrendQuery, outcome::StoredStatus, postgres::repository::PgBenchmarkRepository,
    repository::BenchmarkRepository,
};
use uuid::Uuid;

use super::{
    sample::{self, configuration},
    support::setup,
};

// A UUIDv7 with an exact millisecond timestamp, tail only makes the id unique
fn id(number: u64) -> Uuid {
    let value =
        (u128::from(number * 1000) << 80) | (0x7u128 << 76) | (0b10u128 << 62) | u128::from(number);

    Uuid::from_u128(value)
}

// Stores run `number` under `path`, a failed build makes the run failed
async fn save(repository: &PgBenchmarkRepository, number: u64, path: &str, failed: bool) -> Uuid {
    let mut result = sample::result();

    result.run_id = id(number);
    result.target.path = path.to_string();

    if failed {
        result.metrics.build_time.as_mut().unwrap().status = "failed".to_string();
    }

    repository.save(&result, &configuration()).await.unwrap();

    result.run_id
}

#[tokio::test]
async fn no_runs_means_no_projects_and_zero_totals() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    assert!(repository.list_projects().await.unwrap().is_empty());

    let totals = repository.project_totals("/projects/api").await.unwrap();

    assert_eq!(totals.total_runs, 0);
    assert_eq!(totals.done_runs, 0);

    database.teardown().await;
}

#[tokio::test]
async fn two_interleaved_projects_are_listed_counted_and_trended_apart() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    save(&repository, 1, "/projects/api", false).await;
    save(&repository, 2, "/projects/web", false).await;
    save(&repository, 3, "/projects/api", true).await;
    save(&repository, 4, "/projects/web", false).await;
    save(&repository, 5, "/projects/api", false).await;

    let projects = repository.list_projects().await.unwrap();

    assert_eq!(
        projects
            .iter()
            .map(|project| (project.path.as_str(), project.total_runs, project.done_runs))
            .collect::<Vec<_>>(),
        vec![("/projects/api", 3, 2), ("/projects/web", 2, 2)]
    );
    assert_eq!(projects[0].runtime_code.as_deref(), Some("bun"));

    let api = repository.project_totals("/projects/api").await.unwrap();

    assert_eq!(
        (
            api.total_runs,
            api.done_runs,
            api.failed_runs,
            api.cancelled_runs
        ),
        (3, 2, 1, 0)
    );

    let web = repository.project_totals("/projects/web").await.unwrap();

    assert_eq!((web.total_runs, web.done_runs), (2, 2));

    let trend = |path: &str| TrendQuery {
        limit: 30,
        target_path: Some(path.to_string()),
        status: Some(StoredStatus::Done),
    };

    let api_trend = repository
        .list_trends(&trend("/projects/api"))
        .await
        .unwrap();

    assert_eq!(
        api_trend.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![id(5), id(1)]
    );

    let web_trend = repository
        .list_trends(&trend("/projects/web"))
        .await
        .unwrap();

    assert_eq!(
        web_trend.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![id(4), id(2)]
    );

    database.teardown().await;
}

#[tokio::test]
async fn a_deleted_run_leaves_the_project_list_totals_and_trend() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());

    save(&repository, 1, "/projects/api", false).await;
    let newest = save(&repository, 2, "/projects/api", false).await;
    let only_web = save(&repository, 3, "/projects/web", false).await;

    assert!(repository.delete_run(newest).await.unwrap());
    assert!(repository.delete_run(only_web).await.unwrap());

    let projects = repository.list_projects().await.unwrap();

    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].path, "/projects/api");
    assert_eq!(projects[0].total_runs, 1);
    assert_eq!(projects[0].last_run_at.timestamp_millis(), 1000);

    let totals = repository.project_totals("/projects/api").await.unwrap();

    assert_eq!((totals.total_runs, totals.done_runs), (1, 1));

    let trend = repository
        .list_trends(&TrendQuery {
            limit: 30,
            target_path: Some("/projects/api".to_string()),
            status: Some(StoredStatus::Done),
        })
        .await
        .unwrap();

    assert_eq!(trend.len(), 1);
    assert_eq!(trend[0].id, id(1));

    database.teardown().await;
}
