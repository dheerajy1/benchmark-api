use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use benchmark_api::{
    app,
    config::env::Env,
    persistence::{
        history::SummaryRow, postgres::mapping::run_created_at, repository::BenchmarkRepository,
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

use super::fake_repository::{FailingRepository, InMemoryRepository};

fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

async fn send(
    repository: Arc<dyn BenchmarkRepository>,
    method: Method,
    uri: &str,
    body: Body,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap();

    let response = app::router(test_env(), repository)
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn get(repository: &Arc<InMemoryRepository>, uri: &str) -> (StatusCode, Value) {
    send(repository.clone(), Method::GET, uri, Body::empty()).await
}

fn assert_error_shape(body: &Value, http_code: u64, code: &str) {
    assert_eq!(body["success"], json!(false));
    assert_eq!(body["httpCode"], json!(http_code));
    assert_eq!(body["code"], json!(code));
    assert!(body["message"].is_string());
    assert!(body["error"].is_string());
}

// A UUIDv7 with an exact millisecond timestamp, tail only makes the id unique
fn v7_at(milliseconds: u64, tail: u64) -> Uuid {
    let value = (u128::from(milliseconds) << 80)
        | (0x7u128 << 76)
        | (0b10u128 << 62)
        | u128::from(tail & ((1 << 62) - 1));

    Uuid::from_u128(value)
}

fn summary(id: Uuid, target_path: &str, runtime: Option<&str>) -> SummaryRow {
    SummaryRow {
        id,
        target_path: target_path.to_string(),
        runtime_code: runtime.map(str::to_string),
        created_at: run_created_at(id).unwrap(),
        build_time_ms: Some(1000),
        startup_time_ms: Some(500),
        memory_peak_bytes: Some(2048),
        cpu_average_percent: Some(10.5),
        requests_per_second: Some(100.25),
        p95_ms: Some(8.7),
    }
}

// Seeds `count` runs one second apart, the newest has the highest number
fn seed(repository: &InMemoryRepository, count: u64) -> Vec<Uuid> {
    (1..=count)
        .map(|number| {
            let id = v7_at(number * 1000, number);

            repository.seed_summary(summary(id, "/projects/api", Some("bun")));

            id
        })
        .collect()
}

fn run_ids(body: &Value) -> Vec<String> {
    body["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["run_id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn an_empty_history_returns_an_empty_page() {
    let repository = Arc::new(InMemoryRepository::default());

    let (status, body) = get(&repository, "/api/v1/benchmarks").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["httpCode"], json!(200));
    assert_eq!(body["data"]["items"], json!([]));
    assert_eq!(
        body["data"]["pagination"],
        json!({ "limit": 20, "has_more": false, "next_cursor": null })
    );
}

#[tokio::test]
async fn multiple_runs_are_listed_newest_first_as_summaries() {
    let repository = Arc::new(InMemoryRepository::default());
    let ids = seed(&repository, 3);

    let (status, body) = get(&repository, "/api/v1/benchmarks").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        run_ids(&body),
        vec![ids[2].to_string(), ids[1].to_string(), ids[0].to_string()]
    );

    let item = &body["data"]["items"][0];

    assert_eq!(item["target"], json!({ "path": "/projects/api" }));
    assert_eq!(item["discovery"], json!({ "runtime": "bun" }));
    assert_eq!(item["created_at"], json!("1970-01-01T00:00:03.000Z"));
    assert_eq!(
        item["summary"],
        json!({
            "build_time_ms": 1000,
            "startup_time_ms": 500,
            "memory_peak_bytes": 2048,
            "cpu_average_percent": 10.5,
            "requests_per_second": 100.25,
            "p95_ms": 8.7
        })
    );

    // A summary never carries the full benchmark
    assert!(item.get("metrics").is_none());
}

#[tokio::test]
async fn the_default_limit_is_twenty() {
    let repository = Arc::new(InMemoryRepository::default());
    seed(&repository, 25);

    let (status, body) = get(&repository, "/api/v1/benchmarks").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(run_ids(&body).len(), 20);
    assert_eq!(body["data"]["pagination"]["limit"], json!(20));
    assert_eq!(body["data"]["pagination"]["has_more"], json!(true));
    assert!(body["data"]["pagination"]["next_cursor"].is_string());
}

#[tokio::test]
async fn the_maximum_limit_is_one_hundred() {
    let repository = Arc::new(InMemoryRepository::default());
    seed(&repository, 105);

    let (status, body) = get(&repository, "/api/v1/benchmarks?limit=100").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(run_ids(&body).len(), 100);
    assert_eq!(body["data"]["pagination"]["limit"], json!(100));
    assert_eq!(body["data"]["pagination"]["has_more"], json!(true));
}

#[tokio::test]
async fn a_limit_above_the_maximum_is_rejected() {
    let repository = Arc::new(InMemoryRepository::default());

    let (status, body) = get(&repository, "/api/v1/benchmarks?limit=101").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
}

#[tokio::test]
async fn zero_negative_and_non_numeric_limits_are_rejected() {
    let repository = Arc::new(InMemoryRepository::default());

    for limit in ["0", "-1", "abc", "", "1.5"] {
        let uri = format!("/api/v1/benchmarks?limit={limit}");

        let (status, body) = get(&repository, &uri).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "limit {limit:?}");
        assert_error_shape(&body, 400, "VALIDATION_ERROR");
    }
}

#[tokio::test]
async fn the_runtime_filter_matches_the_runtime_code() {
    let repository = Arc::new(InMemoryRepository::default());
    let bun = v7_at(1000, 1);
    let node = v7_at(2000, 2);
    let unknown = v7_at(3000, 3);

    repository.seed_summary(summary(bun, "/a", Some("bun")));
    repository.seed_summary(summary(node, "/a", Some("node")));
    repository.seed_summary(summary(unknown, "/a", None));

    let (status, body) = get(&repository, "/api/v1/benchmarks?runtime=bun").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(run_ids(&body), vec![bun.to_string()]);
}

#[tokio::test]
async fn an_unknown_runtime_code_is_a_validation_error() {
    let repository = Arc::new(InMemoryRepository::default());

    let (status, body) = get(&repository, "/api/v1/benchmarks?runtime=cobol").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error_shape(&body, 400, "VALIDATION_ERROR");
}

#[tokio::test]
async fn empty_filters_are_validation_errors() {
    let repository = Arc::new(InMemoryRepository::default());

    for uri in [
        "/api/v1/benchmarks?runtime=",
        "/api/v1/benchmarks?target_path=",
    ] {
        let (status, body) = get(&repository, uri).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_error_shape(&body, 400, "VALIDATION_ERROR");
    }
}

#[tokio::test]
async fn the_target_path_filter_is_an_exact_match() {
    let repository = Arc::new(InMemoryRepository::default());
    let exact = v7_at(1000, 1);

    repository.seed_summary(summary(exact, "/projects/api", Some("bun")));
    repository.seed_summary(summary(v7_at(2000, 2), "/projects/api/nested", Some("bun")));
    repository.seed_summary(summary(v7_at(3000, 3), "/projects", Some("bun")));

    let (status, body) = get(
        &repository,
        "/api/v1/benchmarks?target_path=%2Fprojects%2Fapi",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(run_ids(&body), vec![exact.to_string()]);
}

#[tokio::test]
async fn runtime_and_target_path_filters_combine() {
    let repository = Arc::new(InMemoryRepository::default());
    let both = v7_at(1000, 1);

    repository.seed_summary(summary(both, "/a", Some("bun")));
    repository.seed_summary(summary(v7_at(2000, 2), "/a", Some("node")));
    repository.seed_summary(summary(v7_at(3000, 3), "/b", Some("bun")));

    let (status, body) = get(
        &repository,
        "/api/v1/benchmarks?runtime=bun&target_path=%2Fa",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(run_ids(&body), vec![both.to_string()]);
}

#[tokio::test]
async fn the_cursor_walks_every_run_exactly_once() {
    let repository = Arc::new(InMemoryRepository::default());
    let ids = seed(&repository, 5);

    let mut seen = Vec::new();
    let mut uri = "/api/v1/benchmarks?limit=2".to_string();
    let mut pages = 0;

    loop {
        let (status, body) = get(&repository, &uri).await;

        assert_eq!(status, StatusCode::OK);

        seen.extend(run_ids(&body));
        pages += 1;

        let pagination = &body["data"]["pagination"];

        if pagination["has_more"] == json!(false) {
            assert_eq!(pagination["next_cursor"], json!(null));
            break;
        }

        let cursor = pagination["next_cursor"].as_str().unwrap();

        uri = format!("/api/v1/benchmarks?limit=2&before={cursor}");
    }

    let expected: Vec<String> = ids.iter().rev().map(Uuid::to_string).collect();

    assert_eq!(pages, 3);
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn filters_apply_before_the_cursor() {
    let repository = Arc::new(InMemoryRepository::default());

    let bun_old = v7_at(1000, 1);
    let bun_new = v7_at(4000, 4);

    repository.seed_summary(summary(bun_old, "/a", Some("bun")));
    repository.seed_summary(summary(v7_at(2000, 2), "/a", Some("node")));
    repository.seed_summary(summary(v7_at(3000, 3), "/a", Some("node")));
    repository.seed_summary(summary(bun_new, "/a", Some("bun")));

    let (_, first) = get(&repository, "/api/v1/benchmarks?runtime=bun&limit=1").await;

    assert_eq!(run_ids(&first), vec![bun_new.to_string()]);
    assert_eq!(first["data"]["pagination"]["has_more"], json!(true));

    let cursor = first["data"]["pagination"]["next_cursor"].as_str().unwrap();
    let uri = format!("/api/v1/benchmarks?runtime=bun&limit=1&before={cursor}");

    let (_, second) = get(&repository, &uri).await;

    assert_eq!(run_ids(&second), vec![bun_old.to_string()]);
    assert_eq!(second["data"]["pagination"]["has_more"], json!(false));
}

#[tokio::test]
async fn the_cursor_is_opaque() {
    let repository = Arc::new(InMemoryRepository::default());
    let ids = seed(&repository, 3);

    let (_, body) = get(&repository, "/api/v1/benchmarks?limit=2").await;

    let cursor = body["data"]["pagination"]["next_cursor"].as_str().unwrap();
    let last_id = ids[1].to_string();

    assert!(!cursor.contains(&last_id));
    assert!(!cursor.contains("created_at"));
    assert!(Uuid::parse_str(cursor).is_err());
}

#[tokio::test]
async fn malformed_cursors_are_validation_errors() {
    let repository = Arc::new(InMemoryRepository::default());

    let cursors = [
        "".to_string(),
        "not-base64!!".to_string(),
        URL_SAFE_NO_PAD.encode("hello"),
        URL_SAFE_NO_PAD.encode("123.not-a-uuid"),
    ];

    for cursor in cursors {
        let uri = format!("/api/v1/benchmarks?before={cursor}");

        let (status, body) = get(&repository, &uri).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "cursor {cursor:?}");
        assert_error_shape(&body, 400, "VALIDATION_ERROR");
    }
}

#[tokio::test]
async fn runs_with_the_same_created_at_are_ordered_and_paged_by_id() {
    let repository = Arc::new(InMemoryRepository::default());

    let low = v7_at(5000, 1);
    let middle = v7_at(5000, 2);
    let high = v7_at(5000, 3);

    for id in [middle, high, low] {
        repository.seed_summary(summary(id, "/a", Some("bun")));
    }

    let (_, all) = get(&repository, "/api/v1/benchmarks").await;

    let expected = vec![high.to_string(), middle.to_string(), low.to_string()];

    assert_eq!(run_ids(&all), expected);

    let mut seen = Vec::new();
    let mut uri = "/api/v1/benchmarks?limit=1".to_string();

    loop {
        let (_, body) = get(&repository, &uri).await;

        seen.extend(run_ids(&body));

        let pagination = &body["data"]["pagination"];

        if pagination["has_more"] == json!(false) {
            break;
        }

        uri = format!(
            "/api/v1/benchmarks?limit=1&before={}",
            pagination["next_cursor"].as_str().unwrap()
        );
    }

    assert_eq!(seen, expected);
}

#[tokio::test]
async fn nullable_runtime_and_metrics_are_reported_as_null() {
    let repository = Arc::new(InMemoryRepository::default());
    let id = v7_at(1000, 1);

    let mut row = summary(id, "/a", None);

    row.build_time_ms = None;
    row.startup_time_ms = None;
    row.memory_peak_bytes = None;
    row.cpu_average_percent = None;
    row.requests_per_second = None;
    row.p95_ms = None;

    repository.seed_summary(row);

    let (status, body) = get(&repository, "/api/v1/benchmarks").await;

    assert_eq!(status, StatusCode::OK);

    let item = &body["data"]["items"][0];

    assert_eq!(item["discovery"], json!({ "runtime": null }));
    assert_eq!(
        item["summary"],
        json!({
            "build_time_ms": null,
            "startup_time_ms": null,
            "memory_peak_bytes": null,
            "cpu_average_percent": null,
            "requests_per_second": null,
            "p95_ms": null
        })
    );
}

#[tokio::test]
async fn the_detail_matches_the_post_benchmark_data() {
    let repository = Arc::new(InMemoryRepository::default());
    let temp_dir = TempDir::new().unwrap();

    let request = json!({ "path": temp_dir.path().display().to_string() }).to_string();

    let (post_status, post_body) = send(
        repository.clone(),
        Method::POST,
        "/api/v1/benchmarks",
        Body::from(request),
    )
    .await;

    assert_eq!(post_status, StatusCode::OK);

    let run_id = post_body["data"]["run_id"].as_str().unwrap();

    let (status, body) = get(&repository, &format!("/api/v1/benchmarks/{run_id}")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["httpCode"], json!(200));
    assert_eq!(body["data"], post_body["data"]);
}

#[tokio::test]
async fn a_run_that_does_not_exist_is_not_found() {
    let repository = Arc::new(InMemoryRepository::default());
    let uri = format!("/api/v1/benchmarks/{}", Uuid::now_v7());

    let (status, body) = get(&repository, &uri).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error_shape(&body, 404, "NOT_FOUND");
}

#[tokio::test]
async fn a_malformed_run_id_is_a_bad_request() {
    let repository = Arc::new(InMemoryRepository::default());

    for run_id in [
        "not-a-uuid",
        "12345",
        "0192f3a0-zzzz-7000-8000-000000000000",
    ] {
        let (status, body) = get(&repository, &format!("/api/v1/benchmarks/{run_id}")).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "run id {run_id:?}");
        assert_error_shape(&body, 400, "BAD_REQUEST");
    }
}

#[tokio::test]
async fn a_storage_failure_is_an_internal_error() {
    let list = send(
        Arc::new(FailingRepository),
        Method::GET,
        "/api/v1/benchmarks",
        Body::empty(),
    )
    .await;

    assert_eq!(list.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_error_shape(&list.1, 500, "INTERNAL_SERVER_ERROR");

    let uri = format!("/api/v1/benchmarks/{}", Uuid::now_v7());

    let detail = send(
        Arc::new(FailingRepository),
        Method::GET,
        &uri,
        Body::empty(),
    )
    .await;

    assert_eq!(detail.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_error_shape(&detail.1, 500, "INTERNAL_SERVER_ERROR");
}

#[tokio::test]
async fn the_detail_route_rejects_other_methods() {
    let repository = Arc::new(InMemoryRepository::default());
    let uri = format!("/api/v1/benchmarks/{}", Uuid::now_v7());

    for method in [Method::POST, Method::PUT, Method::DELETE] {
        let (status, body) = send(repository.clone(), method, &uri, Body::empty()).await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_error_shape(&body, 405, "METHOD_NOT_ALLOWED");
    }
}
