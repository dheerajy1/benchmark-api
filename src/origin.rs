use axum::http::{HeaderMap, HeaderValue, header::ORIGIN};

// The browser origins that may call the API: the local development ports of the API and
// its tunnels. One list for CORS and for the WebSocket handshake, so they cannot drift.
pub fn is_allowed(origin: &str) -> bool {
    origin == "http://localhost:5002"
        || origin == "http://127.0.0.1:5002"
        || origin == "http://localhost:8092"
        || origin == "http://127.0.0.1:8092"
        || origin.ends_with(".usw3.devtunnels.ms")
        || origin.ends_with(".trycloudflare.com")
}

// A value that is not valid text is never allowed
pub fn is_allowed_header(origin: &HeaderValue) -> bool {
    origin.to_str().map(is_allowed).unwrap_or(false)
}

// The origin check of a WebSocket handshake. CORS does not apply to WebSockets, so the
// server has to look at the Origin header itself:
//
// - no Origin header: allowed. A browser always sends one on a handshake, so a client
//   without it is not a browser (a script, a test, a command line tool). The origin is
//   not authentication, it only keeps other websites from using a visitor's browser.
// - one Origin header on the allow-list: allowed.
// - one Origin header that is not on the list, is `null` or is not valid text: refused.
// - more than one Origin header: refused, the request is malformed.
pub fn websocket_origin_allowed(headers: &HeaderMap) -> bool {
    let mut origins = headers.get_all(ORIGIN).iter();

    match (origins.next(), origins.next()) {
        (None, _) => true,
        (Some(origin), None) => is_allowed_header(origin),
        (Some(_), Some(_)) => false,
    }
}
