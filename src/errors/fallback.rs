use super::{api_error::ApiError, catalog::ErrorCode};

// Router fallback for paths that match no route
pub async fn not_found() -> ApiError {
    ApiError::new(ErrorCode::NotFound, "No route matches the requested path.")
}

// Method fallback for routes that exist but not for this method
pub async fn method_not_allowed() -> ApiError {
    ApiError::new(
        ErrorCode::MethodNotAllowed,
        "This route does not support the requested HTTP method.",
    )
}
