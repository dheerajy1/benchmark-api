use std::net::TcpListener;

use benchmark_api::config::env::Env;
use benchmark_api::features::v1::benchmark::{
    readiness::{self, ReadinessCheck, ReadinessSettings},
    request::ReadinessRequest,
};

use super::test_server::{Behavior, TestServer};

fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

fn settings(port: u16) -> ReadinessSettings {
    ReadinessSettings {
        host: "127.0.0.1".to_string(),
        port,
        path: "/health".to_string(),
        expected_status: 200,
        headers: None,
    }
}

#[test]
fn discovered_settings_use_the_established_defaults() {
    let settings = ReadinessSettings::discovered(5001);

    assert_eq!(settings.host, "127.0.0.1");
    assert_eq!(settings.port, 5001);
    assert_eq!(settings.path, "/api/v1/health/app");
    assert_eq!(settings.expected_status, 200);
    assert!(settings.headers.is_none());
}

#[test]
fn settings_are_built_from_the_request() {
    let request: ReadinessRequest = serde_json::from_value(serde_json::json!({
        "type": "http",
        "host": "localhost",
        "port": 8090,
        "path": "/ping",
        "method": "GET",
        "expected_status": 204,
        "headers": { "X-Custom": "value" }
    }))
    .unwrap();

    let settings = ReadinessSettings::from_request(&request);

    assert_eq!(settings.host, "localhost");
    assert_eq!(settings.port, 8090);
    assert_eq!(settings.path, "/ping");
    assert_eq!(settings.expected_status, 204);
    assert_eq!(
        settings.headers,
        Some(vec![("X-Custom".to_string(), "value".to_string())])
    );
}

#[test]
fn reports_ready_for_the_expected_status() {
    let server = TestServer::start(Behavior::status(200));

    assert_eq!(
        readiness::check(&settings(server.port), &test_env()),
        ReadinessCheck::Ready
    );
}

#[test]
fn reports_rejected_for_another_status() {
    let server = TestServer::start(Behavior::status(500));

    assert_eq!(
        readiness::check(&settings(server.port), &test_env()),
        ReadinessCheck::Rejected
    );
}

#[test]
fn honors_a_custom_expected_status() {
    let server = TestServer::start(Behavior::status(404));

    let mut custom = settings(server.port);

    custom.expected_status = 404;

    assert_eq!(
        readiness::check(&custom, &test_env()),
        ReadinessCheck::Ready
    );
}

#[test]
fn reports_not_ready_when_nothing_listens() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();

    assert_eq!(
        readiness::check(&settings(port), &test_env()),
        ReadinessCheck::NotReady
    );
}

#[test]
fn resolves_localhost() {
    let server = TestServer::start(Behavior::status(200));

    let mut localhost = settings(server.port);

    localhost.host = "localhost".to_string();

    assert_eq!(
        readiness::check(&localhost, &test_env()),
        ReadinessCheck::Ready
    );
}

#[test]
fn sends_env_credentials_when_no_headers_are_given() {
    let server = TestServer::start(Behavior::status(200));

    readiness::check(&settings(server.port), &test_env());

    let heads = server.heads();

    assert_eq!(heads.len(), 1);
    assert!(heads[0].starts_with("get /health http/1.1"));
    assert!(heads[0].contains("x-client-id: test-client-id"));
    assert!(heads[0].contains("x-client-secret: test-client-secret"));
}

#[test]
fn explicit_headers_override_the_env_credentials() {
    let server = TestServer::start(Behavior::status(200));

    let mut explicit = settings(server.port);

    explicit.headers = Some(vec![("X-Custom".to_string(), "custom-value".to_string())]);

    readiness::check(&explicit, &test_env());

    let heads = server.heads();

    assert_eq!(heads.len(), 1);
    assert!(heads[0].contains("x-custom: custom-value"));
    assert!(!heads[0].contains("x-client-id"));
    assert!(!heads[0].contains("x-client-secret"));
}

#[test]
fn empty_explicit_headers_send_no_credentials() {
    let server = TestServer::start(Behavior::status(200));

    let mut explicit = settings(server.port);

    explicit.headers = Some(Vec::new());

    readiness::check(&explicit, &test_env());

    let heads = server.heads();

    assert!(!heads[0].contains("x-client-id"));
}
