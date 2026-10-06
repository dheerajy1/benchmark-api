use axum::{
    Json,
    response::{IntoResponse, Response},
};

use super::{catalog::ErrorCode, response::ErrorBody};

// The one error type handlers return, rendered as the global error shape
#[derive(Debug, Clone)]
pub struct ApiError {
    code: ErrorCode,
    message: String,
    error: String,
}

impl ApiError {
    // Message comes from the catalogue, error carries the specific detail
    pub fn new(code: ErrorCode, error: impl Into<String>) -> Self {
        Self {
            code,
            message: code.message().to_string(),
            error: error.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn error(&self) -> &str {
        &self.error
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.code.http_status();

        let body = ErrorBody {
            success: false,
            http_code: status.as_u16(),
            code: self.code.as_str(),
            message: self.message,
            error: self.error,
        };

        (status, Json(body)).into_response()
    }
}
