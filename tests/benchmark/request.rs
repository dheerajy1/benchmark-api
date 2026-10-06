use benchmark_api::features::v1::benchmark::model::BenchmarkRequest;
use serde_json::json;

#[test]
fn path_only_request_stays_valid() {
    let request: BenchmarkRequest =
        serde_json::from_str(r#"{"path":"~/dev/vs-code/recordlog-api"}"#).unwrap();

    assert_eq!(
        request.path.to_string_lossy(),
        "~/dev/vs-code/recordlog-api"
    );
    assert!(request.startup.is_none());
    assert!(request.workload.is_none());
}

#[test]
fn workload_is_optional() {
    let request: BenchmarkRequest = serde_json::from_value(json!({
        "path": "/tmp/project",
        "startup": {
            "command": { "program": "bun", "args": ["run", "start"] },
            "readiness": {
                "type": "http",
                "host": "127.0.0.1",
                "port": 8090,
                "path": "/api/v1/health/app",
                "method": "GET",
                "expected_status": 200
            }
        }
    }))
    .unwrap();

    assert!(request.startup.is_some());
    assert!(request.workload.is_none());
}

#[test]
fn deserializes_workload_configuration() {
    let request: BenchmarkRequest = serde_json::from_value(json!({
        "path": "/tmp/project",
        "workload": {
            "method": "GET",
            "path": "/api/v1/health/app",
            "headers": {
                "X-Client-Id": "client",
                "X-Client-Secret": "secret"
            },
            "requests": 1000,
            "concurrency": 50
        }
    }))
    .unwrap();

    let workload = request.workload.unwrap();

    assert_eq!(workload.method, "GET");
    assert_eq!(workload.path, "/api/v1/health/app");
    assert_eq!(workload.headers.get("X-Client-Id").unwrap(), "client");
    assert_eq!(workload.headers.get("X-Client-Secret").unwrap(), "secret");
    assert_eq!(workload.requests, 1000);
    assert_eq!(workload.concurrency, 50);
}

#[test]
fn workload_headers_are_optional() {
    let request: BenchmarkRequest = serde_json::from_value(json!({
        "path": "/tmp/project",
        "workload": {
            "method": "GET",
            "path": "/health",
            "requests": 10,
            "concurrency": 2
        }
    }))
    .unwrap();

    assert!(request.workload.unwrap().headers.is_empty());
}

#[test]
fn workload_requires_requests_and_concurrency() {
    let missing_requests = serde_json::from_value::<BenchmarkRequest>(json!({
        "path": "/tmp/project",
        "workload": { "method": "GET", "path": "/health", "concurrency": 2 }
    }));

    let missing_concurrency = serde_json::from_value::<BenchmarkRequest>(json!({
        "path": "/tmp/project",
        "workload": { "method": "GET", "path": "/health", "requests": 10 }
    }));

    assert!(missing_requests.is_err());
    assert!(missing_concurrency.is_err());
}

#[test]
fn deserializes_startup_with_readiness_headers() {
    let request: BenchmarkRequest = serde_json::from_value(json!({
        "path": "/tmp/project",
        "startup": {
            "command": { "program": "bun", "args": ["run", "start"] },
            "readiness": {
                "type": "http",
                "host": "127.0.0.1",
                "port": 8090,
                "path": "/api/v1/health/app",
                "method": "GET",
                "expected_status": 200,
                "headers": { "X-Client-Id": "client" }
            }
        }
    }))
    .unwrap();

    let startup = request.startup.unwrap();

    assert_eq!(startup.command.program, "bun");
    assert_eq!(startup.command.args, ["run", "start"]);
    assert_eq!(startup.readiness.readiness_type, "http");
    assert_eq!(startup.readiness.host, "127.0.0.1");
    assert_eq!(startup.readiness.port, 8090);
    assert_eq!(startup.readiness.path, "/api/v1/health/app");
    assert_eq!(startup.readiness.method, "GET");
    assert_eq!(startup.readiness.expected_status, 200);
    assert_eq!(
        startup
            .readiness
            .headers
            .unwrap()
            .get("X-Client-Id")
            .unwrap(),
        "client"
    );
}

#[test]
fn readiness_headers_are_optional() {
    let request: BenchmarkRequest = serde_json::from_value(json!({
        "path": "/tmp/project",
        "startup": {
            "command": { "program": "bun", "args": [] },
            "readiness": {
                "type": "http",
                "host": "127.0.0.1",
                "port": 8090,
                "path": "/",
                "method": "GET",
                "expected_status": 200
            }
        }
    }))
    .unwrap();

    assert!(request.startup.unwrap().readiness.headers.is_none());
}
