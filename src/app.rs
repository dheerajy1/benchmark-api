use std::sync::Arc;

use axum::{
    Router,
    http::{HeaderName, HeaderValue, Method, header::CONTENT_TYPE},
    middleware,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{
    app_state::AppState,
    config::env::Env,
    errors::fallback,
    features::v1::{
        self,
        benchmark::live::{LiveRuns, RunSlot},
        targets::policy::RunPolicy,
    },
    origin,
    persistence::repository::BenchmarkRepository,
    request_id,
};

// Full application router, shared by main and the API tests. It uses the open policy
// (raw requests allowed, no saved targets), as the router did before saved targets.
pub fn router(env: Env, repository: Arc<dyn BenchmarkRepository>) -> Router {
    let live = LiveRuns::new(env.clone(), Arc::clone(&repository), RunSlot::new());

    router_with_policy(env, repository, live, RunPolicy::open())
}

// The same router around a given live-run service, so a test can share its execution
// slot with the router and hold or inspect it
pub fn router_with_live(
    env: Env,
    repository: Arc<dyn BenchmarkRepository>,
    live: LiveRuns,
) -> Router {
    router_with_policy(env, repository, live, RunPolicy::open())
}

// The router with an explicit run policy, used by the server binary with its settings
pub fn router_with_policy(
    env: Env,
    repository: Arc<dyn BenchmarkRepository>,
    live: LiveRuns,
    policy: RunPolicy,
) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(
            |value: &HeaderValue, _request_parts| origin::is_allowed_header(value),
        ))
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([CONTENT_TYPE])
        // A browser can only read the request id when the server exposes it
        .expose_headers([HeaderName::from_static(request_id::HEADER)]);

    Router::new()
        .merge(v1::router())
        .fallback(fallback::not_found)
        .with_state(AppState {
            env,
            repository,
            live,
            policy: Arc::new(policy),
        })
        // Inside the CORS layer, so every answer of the routes carries an id
        .layer(middleware::from_fn(request_id::assign))
        .layer(cors)
}
