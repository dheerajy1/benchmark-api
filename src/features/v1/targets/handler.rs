use axum::{Router, extract::State, routing::get};

use crate::{
    app_state::AppState,
    errors::{fallback::method_not_allowed, response::ApiSuccess},
};

use super::view::TargetsView;

// GET /api/v1/targets
async fn list(State(state): State<AppState>) -> ApiSuccess<TargetsView> {
    ApiSuccess::ok(
        "Saved targets retrieved.",
        TargetsView::from_policy(&state.policy),
    )
}

pub fn router() -> Router<AppState> {
    Router::new().route("/targets", get(list).fallback(method_not_allowed))
}
