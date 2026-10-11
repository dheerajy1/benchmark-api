// Phase 0 evidence tests for the live-run work. They change nothing in src/.
//
// Part 1 drives the real router over a real TCP socket and observes what the
// legacy POST /api/v1/benchmarks does when the client disconnects mid-run.
//
// Part 2 checks, on the same axum and hyper stack, the permit pattern the
// shared execution guard will use: a value moved into the spawn_blocking
// closure is released when the work ends, not when the client disconnects.
//
// Every test is ignored by default because it is slow and part 1 starts bun.
// Run: cargo test --test legacy_disconnect -- --ignored --nocapture --test-threads=1

use std::{
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use axum::{Router, extract::State, http::StatusCode, routing::post};
use benchmark_api::{app, config::env::Env};
use serde_json::json;
use tempfile::TempDir;

#[allow(dead_code)]
#[path = "benchmark/fake_repository.rs"]
mod fake_repository;

use fake_repository::InMemoryRepository;

// How long the toy blocking work of part 2 runs
const TOY_WORK: Duration = Duration::from_secs(2);

fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

// A real server on a real port, stopped when dropped
struct TestServer {
    address: SocketAddr,
    _runtime: tokio::runtime::Runtime,
}

fn serve(router: Router) -> TestServer {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_io()
        .build()
        .unwrap();

    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();

    runtime.spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    TestServer {
        address,
        _runtime: runtime,
    }
}

fn connect(address: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(address).unwrap();

    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .unwrap();

    stream
}

// Connection: close is only set when the caller wants to read the whole response
fn send_post(stream: &mut TcpStream, path: &str, body: &str, close: bool) {
    let connection = if close { "Connection: close\r\n" } else { "" };

    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n{connection}Content-Length: {}\r\n\r\n{body}",
        body.len()
    );

    stream.write_all(request.as_bytes()).unwrap();
    stream.flush().unwrap();
}

fn read_response(stream: &mut TcpStream) -> String {
    let mut raw = Vec::new();

    stream.read_to_end(&mut raw).unwrap();

    String::from_utf8_lossy(&raw).into_owned()
}

fn disconnect(stream: TcpStream) {
    let _ = stream.shutdown(Shutdown::Both);

    drop(stream);
}

// ---------------------------------------------------------------------------
// Part 1: the real legacy endpoint
// ---------------------------------------------------------------------------

fn require_bun() {
    let available = Command::new("bun")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    assert!(
        available,
        "bun must be on PATH, the existing runtime tests need it too"
    );
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(100),
    )
    .is_ok()
}

// Waits until the port is open (or closed), None when the timeout passes first
fn wait_for_port(port: u16, open: bool, timeout: Duration) -> Option<Instant> {
    let deadline = Instant::now() + timeout;

    loop {
        if port_open(port) == open {
            return Some(Instant::now());
        }

        if Instant::now() >= deadline {
            return None;
        }

        thread::sleep(Duration::from_millis(50));
    }
}

// Explicit startup and a sustained workload, so the run lasts a known time
fn benchmark_body(dir: &Path, port: u16, seconds: u64) -> String {
    let script = format!("Bun.serve({{ port: {port}, fetch() {{ return new Response('ok'); }} }})");

    json!({
        "path": dir.display().to_string(),
        "startup": {
            "command": { "program": "bun", "args": ["-e", script] },
            "readiness": {
                "type": "http",
                "host": "127.0.0.1",
                "port": port,
                "path": "/",
                "method": "GET",
                "expected_status": 200,
                "headers": {}
            }
        },
        "workload": {
            "method": "GET",
            "path": "/",
            "duration_seconds": seconds,
            "concurrency": 2
        }
    })
    .to_string()
}

// Control: proves the harness itself works, so a missing save below is not a harness bug
#[test]
#[ignore = "slow, starts bun, run explicitly"]
fn legacy_client_that_stays_connected_gets_the_result_and_it_is_saved() {
    require_bun();

    let temp_dir = TempDir::new().unwrap();
    let port = free_port();
    let repository = Arc::new(InMemoryRepository::default());
    let server = serve(app::router(test_env(), repository.clone()));

    let mut stream = connect(server.address);

    send_post(
        &mut stream,
        "/api/v1/benchmarks",
        &benchmark_body(temp_dir.path(), port, 2),
        true,
    );

    let response = read_response(&mut stream);
    let saved = repository.saved_run_ids().len();

    println!(
        "PHASE0 legacy_control: status_line={:?} saved_runs={saved}",
        response.lines().next().unwrap_or("")
    );

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("Benchmark completed."), "{response}");
    assert_eq!(saved, 1, "the connected client's result must be saved");
}

// Hypothesis: the run keeps going after the client leaves, but its result is never saved
#[test]
#[ignore = "slow, starts bun, run explicitly"]
fn legacy_client_disconnect_keeps_running_but_discards_the_result() {
    require_bun();

    let temp_dir = TempDir::new().unwrap();
    let port = free_port();
    let repository = Arc::new(InMemoryRepository::default());
    let server = serve(app::router(test_env(), repository.clone()));

    let mut stream = connect(server.address);

    send_post(
        &mut stream,
        "/api/v1/benchmarks",
        &benchmark_body(temp_dir.path(), port, 4),
        false,
    );

    let target_up = wait_for_port(port, true, Duration::from_secs(20))
        .expect("the target never came up, the run did not start");

    thread::sleep(Duration::from_millis(1500));

    assert!(
        port_open(port),
        "the run ended before the disconnect, the scenario is invalid"
    );

    let disconnected = Instant::now();

    disconnect(stream);

    let target_gone = wait_for_port(port, false, Duration::from_secs(40))
        .expect("the target was still running 40 s after the disconnect");

    let ran_on = target_gone.saturating_duration_since(disconnected);

    // Gives a handler that is still alive time to save before counting
    thread::sleep(Duration::from_millis(1500));

    let saved = repository.saved_run_ids().len();

    println!(
        "PHASE0 legacy_disconnect: target_up_to_disconnect_ms={} disconnect_to_target_exit_ms={} saved_runs={saved}",
        disconnected
            .saturating_duration_since(target_up)
            .as_millis(),
        ran_on.as_millis()
    );

    assert!(
        ran_on >= Duration::from_millis(1000),
        "the run did not continue after the disconnect (ran on {} ms)",
        ran_on.as_millis()
    );

    assert_eq!(
        saved, 0,
        "HYPOTHESIS REFUTED: the result was saved although the client disconnected"
    );
}

// ---------------------------------------------------------------------------
// Part 2: the permit pattern on the real axum and hyper stack
// ---------------------------------------------------------------------------

// Named instants, written by the toy handlers and read by the tests
#[derive(Clone, Default)]
struct Timeline(Arc<Mutex<Vec<(&'static str, Instant)>>>);

impl Timeline {
    fn mark(&self, name: &'static str) {
        self.0.lock().unwrap().push((name, Instant::now()));
    }

    fn get(&self, name: &str) -> Option<Instant> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|(recorded, _)| *recorded == name)
            .map(|(_, instant)| *instant)
    }

    fn wait_for(&self, name: &str, timeout: Duration) -> Option<Instant> {
        let deadline = Instant::now() + timeout;

        loop {
            if let Some(instant) = self.get(name) {
                return Some(instant);
            }

            if Instant::now() >= deadline {
                return None;
            }

            thread::sleep(Duration::from_millis(10));
        }
    }
}

// Stand-in for the execution permit, records the moment it is released
struct Permit {
    timeline: Timeline,
}

impl Permit {
    fn new(timeline: &Timeline) -> Self {
        Self {
            timeline: timeline.clone(),
        }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.timeline.mark("permit_released");
    }
}

// The planned design: the permit goes into the blocking closure and comes back with the result
async fn moved_into_closure(State(timeline): State<Timeline>) -> StatusCode {
    let permit = Permit::new(&timeline);
    let worker = timeline.clone();

    let outcome = tokio::task::spawn_blocking(move || {
        worker.mark("work_started");
        thread::sleep(TOY_WORK);
        worker.mark("work_finished");

        permit
    })
    .await;

    match outcome {
        Ok(permit) => {
            drop(permit);

            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// The naive alternative: the permit lives in the handler future
async fn held_by_handler(State(timeline): State<Timeline>) -> StatusCode {
    let _permit = Permit::new(&timeline);
    let worker = timeline.clone();

    let _ = tokio::task::spawn_blocking(move || {
        worker.mark("work_started");
        thread::sleep(TOY_WORK);
        worker.mark("work_finished");
    })
    .await;

    StatusCode::OK
}

// The blocking work panics while it owns the permit
async fn panicking_closure(State(timeline): State<Timeline>) -> StatusCode {
    let permit = Permit::new(&timeline);
    let worker = timeline.clone();

    let outcome = tokio::task::spawn_blocking(move || -> () {
        let _held = permit;

        worker.mark("work_started");
        thread::sleep(Duration::from_millis(200));

        panic!("simulated benchmark panic");
    })
    .await;

    match outcome {
        Ok(()) => StatusCode::OK,
        Err(error) => {
            if error.is_panic() {
                timeline.mark("panic_seen_by_handler");
            }

            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn toy_router(timeline: Timeline) -> Router {
    Router::new()
        .route("/moved", post(moved_into_closure))
        .route("/naive", post(held_by_handler))
        .route("/panic", post(panicking_closure))
        .with_state(timeline)
}

#[test]
#[ignore = "slow, run explicitly"]
fn permit_moved_into_the_blocking_closure_outlives_a_client_disconnect() {
    let timeline = Timeline::default();
    let server = serve(toy_router(timeline.clone()));

    let mut stream = connect(server.address);

    send_post(&mut stream, "/moved", "", false);

    timeline
        .wait_for("work_started", Duration::from_secs(5))
        .expect("the work never started");

    let disconnected = Instant::now();

    disconnect(stream);

    thread::sleep(Duration::from_millis(600));

    assert!(
        timeline.get("permit_released").is_none(),
        "the permit was released while the work was still running"
    );

    let released = timeline
        .wait_for("permit_released", Duration::from_secs(10))
        .expect("the permit was never released");
    let finished = timeline
        .get("work_finished")
        .expect("the work never finished");

    println!(
        "PHASE0 permit_moved: disconnect_to_release_ms={}",
        released.saturating_duration_since(disconnected).as_millis()
    );

    assert!(released >= finished, "released before the work finished");
    assert!(
        released.saturating_duration_since(disconnected) >= Duration::from_millis(1000),
        "released too soon after the disconnect"
    );
}

#[test]
#[ignore = "slow, run explicitly"]
fn permit_held_by_the_handler_is_released_at_disconnect_while_work_continues() {
    let timeline = Timeline::default();
    let server = serve(toy_router(timeline.clone()));

    let mut stream = connect(server.address);

    send_post(&mut stream, "/naive", "", false);

    timeline
        .wait_for("work_started", Duration::from_secs(5))
        .expect("the work never started");

    let disconnected = Instant::now();

    disconnect(stream);

    let released = timeline
        .wait_for("permit_released", Duration::from_secs(1))
        .expect("the handler future was not dropped within 1 s of the disconnect");

    println!(
        "PHASE0 permit_naive: disconnect_to_release_ms={} work_finished_at_release={}",
        released.saturating_duration_since(disconnected).as_millis(),
        timeline.get("work_finished").is_some()
    );

    assert!(
        timeline.get("work_finished").is_none(),
        "the permit was released only after the work, the handler was not cancelled"
    );
}

#[test]
#[ignore = "slow, run explicitly"]
fn permit_is_released_when_the_blocking_work_panics() {
    let timeline = Timeline::default();
    let server = serve(toy_router(timeline.clone()));

    let mut stream = connect(server.address);

    send_post(&mut stream, "/panic", "", true);

    let response = read_response(&mut stream);

    println!(
        "PHASE0 permit_panic: status_line={:?} released={} panic_seen={}",
        response.lines().next().unwrap_or(""),
        timeline.get("permit_released").is_some(),
        timeline.get("panic_seen_by_handler").is_some()
    );

    assert!(response.starts_with("HTTP/1.1 500"), "{response}");
    assert!(
        timeline.get("permit_released").is_some(),
        "the permit was not released after the panic"
    );
    assert!(
        timeline.get("panic_seen_by_handler").is_some(),
        "the handler did not see the panic as a JoinError"
    );
}
