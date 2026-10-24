// Raw requests must answer exactly as they did before saved targets existed. The reference
// route uses the original extractor directly, so every rejection can be compared.

use axum::{Router, http::StatusCode, routing::post};
use benchmark_api::{
    errors::{json::ApiJson, response::ApiSuccess},
    features::v1::benchmark::model::BenchmarkRequest,
};
use serde_json::{Value, json};

use super::support::{post_raw, router, send};
use benchmark_api::features::v1::targets::policy::RunPolicy;

async fn reference(ApiJson(_request): ApiJson<BenchmarkRequest>) -> ApiSuccess<Value> {
    ApiSuccess::ok("reference", json!({}))
}

fn reference_router() -> Router {
    Router::new().route("/reference", post(reference))
}

#[tokio::test]
async fn raw_rejections_match_the_original_extractor() {
    let cases: [(&str, Option<&str>); 7] = [
        ("{", Some("application/json")),
        ("{}", Some("application/json")),
        (r#"{"path": 5}"#, Some("application/json")),
        (r#"{"path": "/x", "startup": 7}"#, Some("application/json")),
        (r#"{"path": "/x"}"#, None),
        (r#"{"path": "/x"}"#, Some("text/plain")),
        ("", Some("application/json")),
    ];

    for (body, content_type) in cases {
        let (app, _) = router(RunPolicy::open());

        let (live_status, live) =
            send(app, post_raw("/api/v1/live-runs", body, content_type)).await;
        let (reference_status, reference) = send(
            reference_router(),
            post_raw("/reference", body, content_type),
        )
        .await;

        assert_eq!(live_status, reference_status, "status for {body:?}");
        assert_eq!(live["code"], reference["code"], "code for {body:?}");
        assert_eq!(
            live["message"], reference["message"],
            "message for {body:?}"
        );
        assert_eq!(live["error"], reference["error"], "error for {body:?}");
    }
}

#[tokio::test]
async fn a_valid_raw_request_is_still_accepted_with_the_open_policy() {
    let dir = tempfile::tempdir().unwrap();
    let body = json!({ "path": dir.path().display().to_string() }).to_string();
    let (app, _) = router(RunPolicy::open());

    let (status, _) = send(
        app,
        post_raw("/api/v1/live-runs", &body, Some("application/json")),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
}
