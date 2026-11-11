use std::sync::Arc;

use benchmark_api::app;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite;

use super::support::{WsClient, serve, test_env};
use crate::fake_repository::InMemoryRepository;

const PATH: &str = "/api/v1/status/ws";

fn start() -> super::support::TestServer {
    let repository = Arc::new(InMemoryRepository::default());

    serve(app::router(test_env(), repository))
}

// The first two frames, in whichever order the server sends them
fn opening(client: &mut WsClient) -> (Value, Value) {
    let first = client.json();
    let second = client.json();

    if first["type"] == "active" {
        (first, second)
    } else {
        (second, first)
    }
}

#[test]
fn a_new_socket_gets_the_active_run_and_the_health() {
    let server = start();
    let mut client = WsClient::open(server.address, PATH, &[]).expect("opens");

    let (active, health) = opening(&mut client);

    assert_eq!(active, json!({ "type": "active", "data": null }));
    assert_eq!(health["type"], "health");
    assert_eq!(health["data"]["ok"], true);
    assert_eq!(health["data"]["database"], true);
    assert!(health["data"]["version"].is_string());
}

#[test]
fn a_ping_is_answered_with_the_same_id() {
    let server = start();
    let mut client = WsClient::open(server.address, PATH, &[]).expect("opens");

    opening(&mut client);

    client.send_text(r#"{"type":"ping","id":7}"#);

    assert_eq!(client.json(), json!({ "type": "pong", "id": 7 }));
}

#[test]
fn other_client_frames_are_ignored() {
    let server = start();
    let mut client = WsClient::open(server.address, PATH, &[]).expect("opens");

    opening(&mut client);

    client.send_text("hello");
    client.send_text(r#"{"type":"ping"}"#);
    client.send_text(r#"{"type":"other","id":1}"#);
    client.send_text(r#"{"type":"ping","id":8}"#);

    assert_eq!(client.json(), json!({ "type": "pong", "id": 8 }));
}

#[test]
fn an_origin_that_is_not_allowed_is_refused_before_the_upgrade() {
    let server = start();

    let refused = WsClient::open(server.address, PATH, &["https://example.com"]);

    match refused {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 403),
        Err(other) => panic!("expected a 403, got {other:?}"),
        Ok(_) => panic!("the origin should have been refused"),
    }
}
