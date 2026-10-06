use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

// { success: false, httpCode, code, message, error }
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub success: bool,
    pub http_code: u16,
    pub code: &'static str,
    pub message: String,
    pub error: String,
}

// { success: true, httpCode, message, data }
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessBody<T> {
    pub success: bool,
    pub http_code: u16,
    pub message: String,
    pub data: T,
}

// Successful handler result wrapped in the global success shape
pub struct ApiSuccess<T> {
    message: String,
    data: T,
}

impl<T> ApiSuccess<T> {
    pub fn ok(message: impl Into<String>, data: T) -> Self {
        Self {
            message: message.into(),
            data,
        }
    }
}

impl<T: Serialize> IntoResponse for ApiSuccess<T> {
    fn into_response(self) -> Response {
        let status = StatusCode::OK;

        let body = SuccessBody {
            success: true,
            http_code: status.as_u16(),
            message: self.message,
            data: self.data,
        };

        (status, Json(body)).into_response()
    }
}
