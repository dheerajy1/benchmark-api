// A value from the environment must reach the outgoing request and nothing else:
// not a response, an event, a listing, an error, or a debug line.

use axum::http::StatusCode;
use benchmark_api::features::v1::targets::{catalog::TargetCatalog, policy::RunPolicy};
use serde_json::json;
use tempfile::tempdir;

use super::support::{
    get_path, lookup, post_json, router, send, targets_json, wait_for_seen, write_file,
};

const CANARY: &str = "CANARY-7f3a9c1e-do-not-leak";

#[tokio::test]
async fn the_secret_value_leaves_only_in_the_outgoing_request() {
    let dir = tempdir().unwrap();
    let file = write_file(dir.path(), "targets.json", &targets_json(dir.path()));
    let lookup = lookup(&[("DEMO_SECRET", CANARY)]);
    let catalog = TargetCatalog::load(&file, &lookup).unwrap();
    let (app, executor) = router(RunPolicy::new(false, catalog.clone()));

    let mut outputs: Vec<String> = Vec::new();

    let (status, accepted) = send(
        app.clone(),
        post_json("/api/v1/live-runs", r#"{"target_id":"demo"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    outputs.push(accepted.to_string());

    let run_id = accepted["data"]["run_id"].as_str().unwrap().to_string();
    wait_for_seen(&executor, 1).await;

    let seen = executor.seen.lock().unwrap().clone();
    let sent = seen[0]
        .workload
        .as_ref()
        .unwrap()
        .headers
        .get("X-Client-Secret")
        .unwrap();

    // Without this the test would pass on an empty value
    assert_eq!(sent, CANARY);
    outputs.push(format!("{:?}", seen));

    let (_, replay) = send(
        app.clone(),
        get_path(&format!("/api/v1/live-runs/{run_id}")),
    )
    .await;
    outputs.push(replay.to_string());

    let (_, listing) = send(app.clone(), get_path("/api/v1/targets")).await;
    outputs.push(listing.to_string());

    let refusals = [
        json!({ "target_id": "demo", "path": CANARY }),
        json!({ "target_id": "demo", "workload": { "concurrency": CANARY } }),
        json!({ "target_id": CANARY }),
        json!({ "target_id": "demo", "headers": { "X-Client-Secret": CANARY } }),
    ];

    for body in refusals {
        let (status, error) = send(
            app.clone(),
            post_json("/api/v1/live-runs", &body.to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        outputs.push(error.to_string());
    }

    outputs.push(format!("{:?}", catalog));

    for output in outputs {
        assert!(
            !output.contains(CANARY),
            "the secret value leaked: {output}"
        );
    }
}
