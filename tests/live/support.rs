use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use benchmark_api::{
    config::env::Env,
    features::v1::benchmark::{
        error::BenchmarkError,
        live::{LiveRuns, RunContext, RunEntry, RunExecutor, RunPermit, RunSlot, Stage},
        model::{BenchmarkRequest, BenchmarkResult},
    },
};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{
    self, Message, WebSocket, client::IntoClientRequest, http::HeaderValue, stream::MaybeTlsStream,
};
use tower::ServiceExt;
use uuid::Uuid;

use crate::comparison_sample;

pub fn test_env() -> Env {
    Env {
        app_port: 5001,
        app_health_client_id: "test-client-id".to_string(),
        app_health_client_secret: "test-client-secret".to_string(),
    }
}

// The smallest valid request: a directory, no startup and no workload
pub fn request_for(dir: &Path) -> BenchmarkRequest {
    serde_json::from_value(json!({ "path": dir.display().to_string() })).unwrap()
}

pub fn post_json(body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/benchmarks")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub async fn send(router: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    (status, serde_json::from_slice(&bytes).unwrap())
}

pub fn kinds(entry: &RunEntry) -> Vec<&'static str> {
    entry
        .replay(0)
        .events
        .iter()
        .map(|event| event.kind)
        .collect()
}

pub async fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);

    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");

        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

pub async fn wait_until_slot_free(slot: &RunSlot) {
    wait_until(|| slot.active().is_none(), "the slot to be free").await;
}

// Waits for the terminal event and checks the guard invariant on every step: the slot
// is released after the terminal event, so a free slot next to a running run can only
// be a violation. The slot is read first so a stale read cannot raise a false alarm.
pub async fn wait_until_terminal(live: &LiveRuns, entry: &RunEntry) {
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        let slot_free = live.slot().active().is_none();
        let terminal = entry.is_terminal();

        assert!(
            !slot_free || terminal,
            "the slot was free while the run was still reported as running"
        );

        if terminal {
            return;
        }

        assert!(Instant::now() < deadline, "the run never became terminal");

        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

// Finishes at once with a complete result
pub struct ImmediateExecutor;

impl RunExecutor for ImmediateExecutor {
    fn execute(
        &self,
        _request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        run_id: Uuid,
        ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        ctx.stage_started(Stage::Build);
        ctx.stage_completed(Stage::Build, "success", Some(1));

        Ok(comparison_sample::result(run_id))
    }
}

// Runs until the test opens the gate
pub struct GatedExecutor {
    pub gate: Arc<AtomicBool>,
}

impl RunExecutor for GatedExecutor {
    fn execute(
        &self,
        _request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        run_id: Uuid,
        ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        ctx.stage_started(Stage::Build);

        while !self.gate.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
        }

        Ok(comparison_sample::result(run_id))
    }
}

pub struct FailingExecutor;

impl RunExecutor for FailingExecutor {
    fn execute(
        &self,
        _request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        _run_id: Uuid,
        ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        ctx.stage_started(Stage::Build);

        Err(BenchmarkError::Internal("boom".to_string()))
    }
}

pub struct PanickingExecutor;

impl RunExecutor for PanickingExecutor {
    fn execute(
        &self,
        _request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        _run_id: Uuid,
        ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        ctx.stage_started(Stage::Startup);

        panic!("executor exploded");
    }
}

pub fn require_bun() {
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

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// Explicit startup and a sustained workload, so the run lasts a known time
pub fn long_benchmark_body(dir: &Path, port: u16, seconds: u64) -> String {
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

// Emits some samples, then runs until the test opens the gate
pub struct SamplingExecutor {
    pub samples: usize,
    pub gate: Arc<AtomicBool>,
}

impl RunExecutor for SamplingExecutor {
    fn execute(
        &self,
        _request: &BenchmarkRequest,
        _target: &Path,
        _env: &Env,
        run_id: Uuid,
        ctx: &RunContext,
        _permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        ctx.stage_started(Stage::Build);

        for index in 0..self.samples {
            ctx.sample(json!({ "n": index }));
        }

        while !self.gate.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
        }

        Ok(comparison_sample::result(run_id))
    }
}

// A real server on a real port. Dropping it opens every registered gate first, so a failing
// test can never hang on a run that waits for its gate.
pub struct TestServer {
    pub address: SocketAddr,
    pub runtime: tokio::runtime::Runtime,
    gates: Vec<Arc<AtomicBool>>,
}

impl TestServer {
    pub fn with_gate(mut self, gate: Arc<AtomicBool>) -> Self {
        self.gates.push(gate);

        self
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        for gate in &self.gates {
            gate.store(true, Ordering::SeqCst);
        }
    }
}

pub fn serve(router: Router) -> TestServer {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
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
        runtime,
        gates: Vec::new(),
    }
}

// What a blocking WebSocket client reads
pub enum Frame {
    Json(Value),
    Close(u16),
    Ping,
    Eof,
    Timeout,
}

pub struct WsClient {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl WsClient {
    // The origins are sent as Origin headers, none sends no header at all
    pub fn open(
        address: SocketAddr,
        path: &str,
        origins: &[&str],
    ) -> Result<Self, tungstenite::Error> {
        let mut request = format!("ws://{address}{path}").into_client_request()?;

        for origin in origins {
            request
                .headers_mut()
                .append("Origin", HeaderValue::from_str(origin).unwrap());
        }

        let (socket, _response) = tungstenite::connect(request)?;

        Ok(Self { socket })
    }

    pub fn next(&mut self, timeout: Duration) -> Frame {
        if let MaybeTlsStream::Plain(stream) = self.socket.get_mut() {
            stream.set_read_timeout(Some(timeout)).unwrap();
        }

        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    return Frame::Json(serde_json::from_str(text.as_str()).unwrap());
                }
                Ok(Message::Close(frame)) => {
                    return Frame::Close(frame.map(|frame| u16::from(frame.code)).unwrap_or(1005));
                }
                Ok(Message::Ping(_)) => return Frame::Ping,
                Ok(_) => continue,
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Frame::Timeout;
                }
                Err(_) => return Frame::Eof,
            }
        }
    }

    pub fn json(&mut self) -> Value {
        match self.next(Duration::from_secs(10)) {
            Frame::Json(value) => value,
            Frame::Close(code) => panic!("expected a JSON frame, got close {code}"),
            Frame::Ping => panic!("expected a JSON frame, got a ping"),
            Frame::Eof => panic!("expected a JSON frame, the socket ended"),
            Frame::Timeout => panic!("expected a JSON frame, timed out"),
        }
    }

    // Every JSON frame up to the close, and the close code (0 when the socket just ended)
    pub fn until_close(&mut self) -> (Vec<Value>, u16) {
        let mut frames = Vec::new();

        loop {
            match self.next(Duration::from_secs(20)) {
                Frame::Json(value) => frames.push(value),
                Frame::Close(code) => return (frames, code),
                Frame::Ping => continue,
                Frame::Eof => return (frames, 0),
                Frame::Timeout => panic!("timed out before the close, got {frames:?}"),
            }
        }
    }

    pub fn send_text(&mut self, text: &str) {
        let _ = self.socket.send(Message::text(text.to_string()));
    }
}

pub fn wait_until_blocking(mut condition: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);

    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");

        thread::sleep(Duration::from_millis(5));
    }
}

pub fn get_json(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

pub fn post_to(path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}
