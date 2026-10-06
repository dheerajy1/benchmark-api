use benchmark_api::features::v1::benchmark::{model::BenchmarkRequest, validation};
use serde_json::{Value, json};

fn startup() -> Value {
    json!({
        "command": { "program": "bun", "args": ["run", "start"] },
        "readiness": {
            "type": "http",
            "host": "127.0.0.1",
            "port": 8090,
            "path": "/api/v1/health/app",
            "method": "GET",
            "expected_status": 200
        }
    })
}

fn workload() -> Value {
    json!({
        "method": "GET",
        "path": "/api/v1/health/app",
        "headers": { "X-Client-Id": "client" },
        "requests": 1000,
        "concurrency": 50
    })
}

fn request(startup: Value, workload: Value) -> BenchmarkRequest {
    serde_json::from_value(json!({
        "path": "/tmp/project",
        "startup": startup,
        "workload": workload
    }))
    .unwrap()
}

fn error_of(request: &BenchmarkRequest) -> String {
    validation::validate(request).err().unwrap()
}

#[test]
fn accepts_path_only_request() {
    let request: BenchmarkRequest =
        serde_json::from_value(json!({ "path": "/tmp/project" })).unwrap();

    assert!(validation::validate(&request).is_ok());
}

#[test]
fn accepts_full_request() {
    assert!(validation::validate(&request(startup(), workload())).is_ok());
}

#[test]
fn accepts_workload_without_startup() {
    assert!(validation::validate(&request(Value::Null, workload())).is_ok());
}

#[test]
fn accepts_concurrency_above_requests() {
    let mut workload = workload();

    workload["requests"] = json!(10);
    workload["concurrency"] = json!(50);

    assert!(validation::validate(&request(Value::Null, workload)).is_ok());
}

#[test]
fn rejects_zero_requests() {
    let mut workload = workload();

    workload["requests"] = json!(0);

    let error = error_of(&request(Value::Null, workload));

    assert!(error.contains("requests"));
}

#[test]
fn rejects_zero_concurrency() {
    let mut workload = workload();

    workload["concurrency"] = json!(0);

    let error = error_of(&request(Value::Null, workload));

    assert!(error.contains("concurrency"));
}

#[test]
fn rejects_non_get_workload_method() {
    let mut workload = workload();

    workload["method"] = json!("POST");

    assert!(error_of(&request(Value::Null, workload)).contains("GET"));
}

#[test]
fn rejects_workload_path_without_leading_slash() {
    let mut workload = workload();

    workload["path"] = json!("api/v1/health/app");

    assert!(error_of(&request(Value::Null, workload)).contains("path"));
}

#[test]
fn rejects_invalid_workload_header_name() {
    let mut workload = workload();

    workload["headers"] = json!({ "Bad Header": "value" });

    assert!(error_of(&request(Value::Null, workload)).contains("header"));
}

#[test]
fn rejects_invalid_workload_header_value() {
    let mut workload = workload();

    workload["headers"] = json!({ "X-Client-Id": "line1\r\nInjected: yes" });

    assert!(error_of(&request(Value::Null, workload)).contains("header"));
}

#[test]
fn rejects_invalid_readiness_header() {
    let mut startup = startup();

    startup["readiness"]["headers"] = json!({ "X-Client-Id": "bad\nvalue" });

    assert!(error_of(&request(startup, Value::Null)).contains("header"));
}

#[test]
fn rejects_non_loopback_readiness_host() {
    let mut startup = startup();

    startup["readiness"]["host"] = json!("example.com");

    assert!(error_of(&request(startup, Value::Null)).contains("loopback"));
}

#[test]
fn accepts_localhost_and_ipv6_loopback_hosts() {
    for host in ["localhost", "::1", "127.0.0.1"] {
        let mut startup = startup();

        startup["readiness"]["host"] = json!(host);

        assert!(validation::validate(&request(startup, Value::Null)).is_ok());
    }
}

#[test]
fn rejects_unsupported_readiness_type() {
    let mut startup = startup();

    startup["readiness"]["type"] = json!("tcp");

    assert!(error_of(&request(startup, Value::Null)).contains("readiness type"));
}

#[test]
fn rejects_non_get_readiness_method() {
    let mut startup = startup();

    startup["readiness"]["method"] = json!("POST");

    assert!(error_of(&request(startup, Value::Null)).contains("GET"));
}

#[test]
fn rejects_empty_program() {
    let mut startup = startup();

    startup["command"]["program"] = json!("  ");

    assert!(error_of(&request(startup, Value::Null)).contains("program"));
}

#[test]
fn rejects_zero_readiness_port() {
    let mut startup = startup();

    startup["readiness"]["port"] = json!(0);

    assert!(error_of(&request(startup, Value::Null)).contains("port"));
}

#[test]
fn rejects_readiness_path_without_leading_slash() {
    let mut startup = startup();

    startup["readiness"]["path"] = json!("health");

    assert!(error_of(&request(startup, Value::Null)).contains("path"));
}

#[test]
fn rejects_out_of_range_expected_status() {
    let mut startup = startup();

    startup["readiness"]["expected_status"] = json!(42);

    assert!(error_of(&request(startup, Value::Null)).contains("expected_status"));
}

fn duration_workload(seconds: Value) -> Value {
    json!({
        "method": "GET",
        "path": "/api/v1/health/app",
        "headers": {},
        "duration_seconds": seconds,
        "concurrency": 50
    })
}

#[test]
fn accepts_duration_mode() {
    assert!(validation::validate(&request(Value::Null, duration_workload(json!(30)))).is_ok());
}

#[test]
fn accepts_the_maximum_duration() {
    assert!(validation::validate(&request(Value::Null, duration_workload(json!(300)))).is_ok());
}

#[test]
fn rejects_duration_above_the_maximum() {
    let error = error_of(&request(Value::Null, duration_workload(json!(301))));

    assert!(error.contains("duration_seconds"));
    assert!(error.contains("300"));
}

#[test]
fn rejects_zero_duration() {
    let error = error_of(&request(Value::Null, duration_workload(json!(0))));

    assert!(error.contains("duration_seconds"));
}

#[test]
fn rejects_requests_together_with_duration() {
    let mut workload = workload();

    workload["duration_seconds"] = json!(5);

    let error = error_of(&request(Value::Null, workload));

    assert!(error.contains("mutually exclusive"));
}

#[test]
fn rejects_workload_with_neither_requests_nor_duration() {
    let mut workload = workload();

    workload.as_object_mut().unwrap().remove("requests");

    let error = error_of(&request(Value::Null, workload));

    assert!(error.contains("either"));
}

#[test]
fn rejects_zero_concurrency_in_duration_mode() {
    let mut workload = duration_workload(json!(30));

    workload["concurrency"] = json!(0);

    assert!(error_of(&request(Value::Null, workload)).contains("concurrency"));
}

#[test]
fn duration_mode_keeps_the_other_workload_rules() {
    let mut post = duration_workload(json!(30));

    post["method"] = json!("POST");

    assert!(error_of(&request(Value::Null, post)).contains("GET"));

    let mut no_slash = duration_workload(json!(30));

    no_slash["path"] = json!("health");

    assert!(error_of(&request(Value::Null, no_slash)).contains("path"));
}
