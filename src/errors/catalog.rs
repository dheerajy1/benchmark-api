use axum::http::StatusCode;

// Central catalogue of every error code the API can return
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    BadRequest,
    ValidationError,
    NotFound,
    MethodNotAllowed,
    Conflict,
    InternalServerError,
}

impl ErrorCode {
    // Machine readable code sent in the response body
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "BAD_REQUEST",
            ErrorCode::ValidationError => "VALIDATION_ERROR",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::MethodNotAllowed => "METHOD_NOT_ALLOWED",
            ErrorCode::Conflict => "CONFLICT",
            ErrorCode::InternalServerError => "INTERNAL_SERVER_ERROR",
        }
    }

    pub fn http_status(self) -> StatusCode {
        match self {
            ErrorCode::BadRequest | ErrorCode::ValidationError => StatusCode::BAD_REQUEST,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::Conflict => StatusCode::CONFLICT,
            ErrorCode::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // Short generic summary, the specific detail goes in the error field
    pub fn message(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "Some request information is invalid or malformed.",
            ErrorCode::ValidationError => "Request validation failed.",
            ErrorCode::NotFound => "The requested resource was not found.",
            ErrorCode::MethodNotAllowed => "This method is not allowed for the requested resource.",
            ErrorCode::Conflict => "Another benchmark is already running.",
            ErrorCode::InternalServerError => "Something went wrong on the server.",
        }
    }
}
