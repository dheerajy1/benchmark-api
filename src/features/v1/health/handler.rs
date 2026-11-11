use axum::{Router, extract::State, routing::get};
use serde::Serialize;

use crate::{
    app_state::AppState,
    errors::{
        api_error::ApiError, catalog::ErrorCode, fallback::method_not_allowed, response::ApiSuccess,
    },
};

#[derive(Serialize)]
pub struct Health {
    pub ok: bool,
    // The database answered a query
    pub database: bool,
    pub version: &'static str,
}

// Asks the database once, the error says why it cannot be reached
pub async fn check(state: &AppState) -> Result<Health, String> {
    state
        .repository
        .ping()
        .await
        .map_err(|error| format!("The database cannot be reached: {error}"))?;

    Ok(Health {
        ok: true,
        database: true,
        version: env!("CARGO_PKG_VERSION"),
    })
}

// GET /api/v1/health, answers 503 when the database cannot be reached
async fn health(State(state): State<AppState>) -> Result<ApiSuccess<Health>, ApiError> {
    let health = check(&state)
        .await
        .map_err(|message| ApiError::new(ErrorCode::ServiceUnavailable, message))?;

    Ok(ApiSuccess::ok("The API is healthy.", health))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/health", get(health).fallback(method_not_allowed))
}
