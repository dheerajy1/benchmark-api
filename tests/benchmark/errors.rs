use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};
use benchmark_api::{
    errors::{
        api_error::ApiError,
        catalog::ErrorCode,
        response::{ApiSuccess, ErrorBody, SuccessBody},
    },
    features::v1::benchmark::error::BenchmarkError,
};
use serde_json::{Value, json};

async fn body_of(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn catalogue_maps_codes_to_names_and_statuses() {
    let expected = [
        (
            ErrorCode::BadRequest,
            "BAD_REQUEST",
            StatusCode::BAD_REQUEST,
        ),
        (
            ErrorCode::ValidationError,
            "VALIDATION_ERROR",
            StatusCode::BAD_REQUEST,
        ),
        (ErrorCode::NotFound, "NOT_FOUND", StatusCode::NOT_FOUND),
        (
            ErrorCode::MethodNotAllowed,
            "METHOD_NOT_ALLOWED",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            ErrorCode::InternalServerError,
            "INTERNAL_SERVER_ERROR",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ];

    for (code, name, status) in expected {
        assert_eq!(code.as_str(), name);
        assert_eq!(code.http_status(), status);
        assert!(!code.message().is_empty());
    }
}

#[test]
fn api_error_takes_its_message_from_the_catalogue() {
    let error = ApiError::new(ErrorCode::BadRequest, "port must be a number");

    assert_eq!(error.code(), ErrorCode::BadRequest);
    assert_eq!(error.message(), ErrorCode::BadRequest.message());
    assert_eq!(error.error(), "port must be a number");
}

#[tokio::test]
async fn api_error_renders_the_global_error_shape() {
    let response = ApiError::new(
        ErrorCode::ValidationError,
        "requests must be greater than 0",
    )
    .into_response();

    let (status, body) = body_of(response).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({
            "success": false,
            "httpCode": 400,
            "code": "VALIDATION_ERROR",
            "message": "Request validation failed.",
            "error": "requests must be greater than 0"
        })
    );
}

#[tokio::test]
async fn api_success_renders_the_global_success_shape() {
    let response = ApiSuccess::ok("Done.", json!({ "value": 1 })).into_response();

    let (status, body) = body_of(response).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "success": true,
            "httpCode": 200,
            "message": "Done.",
            "data": { "value": 1 }
        })
    );
}

#[test]
fn body_types_serialize_with_camel_case_keys() {
    let error = serde_json::to_value(ErrorBody {
        success: false,
        http_code: 500,
        code: "INTERNAL_SERVER_ERROR",
        message: "m".to_string(),
        error: "e".to_string(),
    })
    .unwrap();

    let success = serde_json::to_value(SuccessBody {
        success: true,
        http_code: 200,
        message: "m".to_string(),
        data: 1,
    })
    .unwrap();

    assert!(error.get("httpCode").is_some());
    assert!(success.get("httpCode").is_some());
}

#[test]
fn benchmark_errors_map_to_catalogue_codes() {
    let validation: ApiError = BenchmarkError::Validation("bad".to_string()).into();
    let path: ApiError = BenchmarkError::InvalidPath("missing".to_string()).into();
    let internal: ApiError = BenchmarkError::Internal("boom".to_string()).into();

    assert_eq!(validation.code(), ErrorCode::ValidationError);
    assert_eq!(path.code(), ErrorCode::BadRequest);
    assert_eq!(internal.code(), ErrorCode::InternalServerError);

    assert_eq!(validation.error(), "bad");
    assert_eq!(path.error(), "missing");
    assert_eq!(internal.error(), "boom");
}

#[test]
fn plain_string_errors_become_internal_benchmark_errors() {
    let error: BenchmarkError = "something broke".to_string().into();

    assert_eq!(
        error,
        BenchmarkError::Internal("something broke".to_string())
    );
    assert_eq!(error.to_string(), "something broke");
}
