use std::time::Instant;

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

pub const HEADER: &str = "x-request-id";

tokio::task_local! {
    // The id of the request being answered, read when an error body is built
    static REQUEST_ID: String;
}

// The id of the request this code runs for, None outside a request
pub fn current() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}

// A short id: 8 lowercase hex characters from the random part of a UUIDv7
pub fn generate() -> String {
    let simple = Uuid::now_v7().simple().to_string();

    simple[simple.len() - 8..].to_string()
}

// Gives every request its own id, returns it in `x-request-id` and logs a line for every
// failed request. An id sent by the client is never used.
pub async fn assign(request: Request, next: Next) -> Response {
    let id = generate();
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let started = Instant::now();

    let mut response = REQUEST_ID.scope(id.clone(), next.run(request)).await;

    if let Ok(value) = HeaderValue::from_str(&id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(HEADER), value);
    }

    let status = response.status();

    if status.as_u16() >= 400 {
        eprintln!(
            "request failed: id={id} method={method} path={path} status={} elapsed_ms={}",
            status.as_u16(),
            started.elapsed().as_millis()
        );
    }

    response
}
