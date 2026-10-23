use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{Router, http::StatusCode};
use benchmark_api::{
    app,
    features::v1::benchmark::live::{LiveRuns, RunExecutor, RunRegistry, RunSlot},
    persistence::repository::BenchmarkRepository,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::support::{
    CancelIgnoringExecutor, CancellableExecutor, FailingExecutor, ImmediateExecutor,
    SlowRepository, get_json, post_to, send, test_env, wait_until, wait_until_slot_free,
};
use crate::fake_repository::InMemoryRepository;

fn router_with(
    repository: Arc<dyn BenchmarkRepository>,
    executor: Arc<dyn RunExecutor>,
) -> (Router, LiveRuns) {
    let live = LiveRuns::with_parts(
        test_env(),
        repository.clone(),
        RunSlot::new(),
        RunRegistry::new(),
        executor,
    );

    let router = app::router_with_live(test_env(), repository, live.clone());

    (router, live)
}

fn valid_body(dir: &TempDir) -> String {
    json!({ "path": dir.path().display().to_string() }).to_string()
}

// Starts a run over HTTP, returns its id and its cancel token
async fn start(router: &Router, dir: &TempDir) -> (Uuid, String) {
    let (status, body) = send(
        router.clone(),
        post_to("/api/v1/live-runs", &valid_body(dir)),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED, "{body}");

    (
        Uuid::parse_str(body["data"]["run_id"].as_str().unwrap()).unwrap(),
        body["data"]["cancel_token"].as_str().unwrap().to_string(),
    )
}

async fn cancel(router: &Router, run_id: Uuid, body: &str) -> (StatusCode, Value) {
    send(
        router.clone(),
        post_to(&format!("/api/v1/live-runs/{run_id}/cancel"), body),
    )
    .await
}

async fn replay(router: &Router, run_id: Uuid) -> Value {
    let (status, body) = send(
        router.clone(),
        get_json(&format!("/api/v1/live-runs/{run_id}")),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");

    body["data"].clone()
}

fn event_kinds(replay: &Value) -> Vec<String> {
    replay["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["type"].as_str().unwrap().to_string())
        .collect()
}

fn assert_error(body: &Value, http_code: u64, code: &str) {
    assert_eq!(body["success"], json!(false), "{body}");
    assert_eq!(body["httpCode"], json!(http_code));
    assert_eq!(body["code"], json!(code));
    assert!(body["message"].is_string());
    assert!(body["error"].is_string());
}

async fn wait_started_in(live: &LiveRuns, run_id: Uuid) {
    wait_until(
        || {
            live.registry().get(run_id).is_some_and(|entry| {
                entry
                    .replay(0)
                    .events
                    .iter()
                    .any(|event| event.kind == "stage.started")
            })
        },
        "the run to start working",
    )
    .await;
}

async fn wait_status(live: &LiveRuns, run_id: Uuid, status: &'static str) {
    wait_until(
        || live.registry_status(run_id) == Some(status),
        "the run to reach its status",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_202_carries_a_cancel_token_and_the_cancel_capability_and_the_token_goes_nowhere_else()
{
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let (router, live) = router_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let (status, body) = send(
        router.clone(),
        post_to("/api/v1/live-runs", &valid_body(&dir)),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);

    let token = body["data"]["cancel_token"].as_str().unwrap().to_string();
    let run_id = Uuid::parse_str(body["data"]["run_id"].as_str().unwrap()).unwrap();

    assert_eq!(token.len(), 32);
    assert!(token.chars().all(|character| character.is_ascii_hexdigit()));
    assert_eq!(body["data"]["capabilities"], json!({ "cancel": true }));

    // The run says it can be cancelled, and nothing a reader can fetch contains the token
    let replay = replay(&router, run_id).await;

    assert_eq!(
        replay["events"][0]["data"]["capabilities"],
        json!({ "cancel": true })
    );
    assert!(!replay.to_string().contains(&token));

    gate.store(true, Ordering::SeqCst);

    wait_status(&live, run_id, "completed").await;
    wait_until_slot_free(live.slot()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_over_http_stops_the_run_and_nothing_is_stored() {
    let dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());
    let gate = Arc::new(AtomicBool::new(false));

    let (router, live) = router_with(
        repository.clone(),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let (run_id, token) = start(&router, &dir).await;

    wait_started_in(&live, run_id).await;

    let body = json!({ "cancel_token": token }).to_string();

    let (status, first) = cancel(&router, run_id, &body).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{first}");
    assert_eq!(first["success"], json!(true));
    assert_eq!(first["httpCode"], json!(202));
    assert_eq!(
        first["data"],
        json!({ "state": "cancelling", "already_requested": false })
    );

    // A duplicate, even after the run has ended, is answered consistently
    let (status, second) = cancel(&router, run_id, &body).await;

    assert!(
        status == StatusCode::ACCEPTED || status == StatusCode::CONFLICT,
        "{status} {second}"
    );

    wait_status(&live, run_id, "cancelled").await;
    wait_until_slot_free(live.slot()).await;

    let replay = replay(&router, run_id).await;
    let names = event_kinds(&replay);

    assert_eq!(replay["status"], json!("cancelled"));
    assert_eq!(names.iter().filter(|n| *n == "run.cancelling").count(), 1);
    assert_eq!(names.iter().filter(|n| *n == "run.cancelled").count(), 1);
    assert_eq!(names.last().unwrap(), "run.cancelled");
    assert_eq!(
        replay["events"].as_array().unwrap().last().unwrap()["data"]["persisted"],
        json!(false)
    );
    assert!(!replay.to_string().contains(&token));

    // Nothing was saved, so the history has no such run
    assert!(repository.saved_run_ids().is_empty());

    let (status, _) = send(
        router.clone(),
        get_json(&format!("/api/v1/benchmarks/{run_id}")),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_cancel_while_cancelling_is_accepted_and_marked_as_a_duplicate() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    // This execution keeps going until its gate opens, so the run stays in "cancelling"
    let (router, live) = router_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(CancelIgnoringExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let (run_id, token) = start(&router, &dir).await;

    wait_started_in(&live, run_id).await;

    let body = json!({ "cancel_token": token }).to_string();

    let (_, first) = cancel(&router, run_id, &body).await;
    let (status, second) = cancel(&router, run_id, &body).await;

    assert_eq!(first["data"]["already_requested"], json!(false));
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(second["data"]["already_requested"], json!(true));

    let replay = replay(&router, run_id).await;

    assert_eq!(replay["status"], json!("running"));
    assert_eq!(
        event_kinds(&replay)
            .iter()
            .filter(|n| *n == "run.cancelling")
            .count(),
        1
    );

    gate.store(true, Ordering::SeqCst);

    wait_status(&live, run_id, "cancelled").await;
    wait_until_slot_free(live.slot()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_wrong_or_empty_token_is_refused_with_403_and_the_run_is_left_alone() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let (router, live) = router_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let (run_id, token) = start(&router, &dir).await;

    wait_started_in(&live, run_id).await;

    for body in [
        r#"{"cancel_token":"wrong"}"#,
        r#"{"cancel_token":""}"#,
        r#"{"cancel_token":null}"#,
        r#"{}"#,
    ] {
        let (status, error) = cancel(&router, run_id, body).await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_error(&error, 403, "FORBIDDEN");
        assert!(!error.to_string().contains(&token));
    }

    // Nothing happened to the run
    let replay = replay(&router, run_id).await;

    assert_eq!(replay["status"], json!("running"));
    assert!(!event_kinds(&replay).contains(&"run.cancelling".to_string()));

    gate.store(true, Ordering::SeqCst);

    wait_status(&live, run_id, "completed").await;
    wait_until_slot_free(live.slot()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_of_another_run_does_not_cancel_this_one() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let (router, live) = router_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let (first_id, first_token) = start(&router, &dir).await;

    gate.store(true, Ordering::SeqCst);
    wait_status(&live, first_id, "completed").await;
    wait_until_slot_free(live.slot()).await;

    gate.store(false, Ordering::SeqCst);

    let (second_id, second_token) = start(&router, &dir).await;

    wait_started_in(&live, second_id).await;

    let (status, error) = cancel(
        &router,
        second_id,
        &json!({ "cancel_token": first_token }).to_string(),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_error(&error, 403, "FORBIDDEN");
    assert_ne!(first_token, second_token);

    gate.store(true, Ordering::SeqCst);

    wait_status(&live, second_id, "completed").await;
    wait_until_slot_free(live.slot()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_runs_malformed_requests_and_other_methods_are_answered_in_the_error_shape() {
    let (router, _live) = router_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(ImmediateExecutor),
    );

    let (status, body) = cancel(&router, Uuid::now_v7(), r#"{"cancel_token":"abc"}"#).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&body, 404, "NOT_FOUND");

    let (status, body) = send(
        router.clone(),
        post_to(
            "/api/v1/live-runs/not-a-uuid/cancel",
            r#"{"cancel_token":"abc"}"#,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, 400, "BAD_REQUEST");

    let (status, body) = cancel(&router, Uuid::now_v7(), "{ not json").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&body, 400, "BAD_REQUEST");

    let (status, body) = send(
        router,
        get_json(&format!("/api/v1/live-runs/{}/cancel", Uuid::now_v7())),
    )
    .await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_error(&body, 405, "METHOD_NOT_ALLOWED");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_run_answers_409_with_how_it_finished() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    for (executor, expected, word) in [
        (
            Arc::new(ImmediateExecutor) as Arc<dyn RunExecutor>,
            "completed",
            "completed",
        ),
        (
            Arc::new(FailingExecutor) as Arc<dyn RunExecutor>,
            "failed",
            "failed",
        ),
        (
            Arc::new(CancellableExecutor {
                gate: Arc::clone(&gate),
            }) as Arc<dyn RunExecutor>,
            "cancelled",
            "cancelled",
        ),
    ] {
        let (router, live) = router_with(Arc::new(InMemoryRepository::default()), executor);

        let (run_id, token) = start(&router, &dir).await;
        let body = json!({ "cancel_token": token }).to_string();

        if expected == "cancelled" {
            wait_started_in(&live, run_id).await;
            cancel(&router, run_id, &body).await;
        }

        wait_status(&live, run_id, expected).await;
        wait_until_slot_free(live.slot()).await;

        let events_before = replay(&router, run_id).await["events"]
            .as_array()
            .unwrap()
            .len();

        let (status, error) = cancel(&router, run_id, &body).await;

        assert_eq!(status, StatusCode::CONFLICT, "{expected}");
        assert_error(&error, 409, "CONFLICT");
        assert!(error["error"].as_str().unwrap().contains(word), "{error}");

        let events_after = replay(&router, run_id).await["events"]
            .as_array()
            .unwrap()
            .len();

        assert_eq!(events_before, events_after);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_is_saving_answers_409_and_completes() {
    let dir = TempDir::new().unwrap();
    let repository = Arc::new(SlowRepository::new(Duration::from_millis(400)));

    let (router, live) = router_with(repository.clone(), Arc::new(ImmediateExecutor));

    let (run_id, token) = start(&router, &dir).await;

    wait_until(
        || {
            live.registry().get(run_id).is_some_and(|entry| {
                entry
                    .replay(0)
                    .events
                    .iter()
                    .any(|event| event.data["stage"] == json!("saving"))
            })
        },
        "the run to start saving",
    )
    .await;

    let (status, error) = cancel(
        &router,
        run_id,
        &json!({ "cancel_token": token }).to_string(),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&error, 409, "CONFLICT");
    assert!(
        error["error"].as_str().unwrap().contains("saving"),
        "{error}"
    );

    wait_status(&live, run_id, "completed").await;
    wait_until_slot_free(live.slot()).await;

    assert_eq!(repository.saved_run_ids(), vec![run_id]);
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_cancellation_over_http_the_next_run_is_accepted_at_once() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let (router, live) = router_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let (first, token) = start(&router, &dir).await;

    wait_started_in(&live, first).await;
    cancel(
        &router,
        first,
        &json!({ "cancel_token": token }).to_string(),
    )
    .await;
    wait_status(&live, first, "cancelled").await;

    // The terminal event comes before the release, so wait for the slot like any client would
    // retry after a 409
    wait_until_slot_free(live.slot()).await;

    gate.store(true, Ordering::SeqCst);

    let (second, _) = start(&router, &dir).await;

    wait_status(&live, second, "completed").await;
    wait_until_slot_free(live.slot()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_legacy_endpoint_is_not_affected_by_cancellation() {
    let dir = TempDir::new().unwrap();

    let (router, live) = router_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(ImmediateExecutor),
    );

    let (status, body) = send(router, post_to("/api/v1/benchmarks", &valid_body(&dir))).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], json!("Benchmark completed."));
    assert!(body["data"].get("cancel_token").is_none());
    assert!(live.slot().active().is_none());
}
