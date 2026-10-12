use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use benchmark_api::{
    app,
    features::v1::benchmark::live::{
        LiveRuns, RunExecutor, RunRegistry, RunSlot, WsConfig, event::kind,
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio_tungstenite::tungstenite;
use uuid::Uuid;

use super::support::{
    Frame, GatedExecutor, ImmediateExecutor, SamplingExecutor, TestServer, WsClient, request_for,
    serve, test_env, wait_until_blocking,
};
use crate::fake_repository::InMemoryRepository;

struct Started {
    server: TestServer,
    live: LiveRuns,
    repository: Arc<InMemoryRepository>,
}

fn start(
    executor: Arc<dyn RunExecutor>,
    registry: Arc<RunRegistry>,
    ws: WsConfig,
    gate: Option<Arc<AtomicBool>>,
) -> Started {
    let repository = Arc::new(InMemoryRepository::default());

    let live = LiveRuns::with_parts(
        test_env(),
        repository.clone(),
        RunSlot::new(),
        registry,
        executor,
    )
    .with_ws_config(ws);

    let router = app::router_with_live(test_env(), repository.clone(), live.clone());

    let mut server = serve(router);

    if let Some(gate) = gate {
        server = server.with_gate(gate);
    }

    Started {
        server,
        live,
        repository,
    }
}

fn gated(ws: WsConfig) -> (Started, Arc<AtomicBool>) {
    let gate = Arc::new(AtomicBool::new(false));

    let started = start(
        Arc::new(GatedExecutor {
            gate: Arc::clone(&gate),
        }),
        RunRegistry::new(),
        ws,
        Some(Arc::clone(&gate)),
    );

    (started, gate)
}

fn submit(
    started: &Started,
    dir: &TempDir,
) -> (
    Uuid,
    Arc<benchmark_api::features::v1::benchmark::live::RunEntry>,
) {
    let accepted = started
        .server
        .runtime
        .block_on(async { started.live.submit(request_for(dir.path())) })
        .ok()
        .expect("accepted");

    (accepted.run_id, accepted.entry)
}

fn ws_path(run_id: Uuid) -> String {
    format!("/api/v1/live-runs/{run_id}/ws")
}

fn open(started: &Started, run_id: Uuid) -> WsClient {
    WsClient::open(started.server.address, &ws_path(run_id), &[])
        .ok()
        .expect("the handshake works")
}

fn types(frames: &[Value]) -> Vec<String> {
    frames
        .iter()
        .map(|frame| frame["type"].as_str().unwrap().to_string())
        .collect()
}

fn seqs(frames: &[Value]) -> Vec<u64> {
    frames
        .iter()
        .filter(|frame| frame["seq"].is_u64())
        .map(|frame| frame["seq"].as_u64().unwrap())
        .collect()
}

// The documented client rule over the frames of one socket: control frames announce the
// highest evicted sample, a gap in seq is legitimate only below it
fn follows_client_rule(frames: &[Value], after: u64) -> bool {
    let mut last = after;
    let mut dropped = 0;

    for frame in frames {
        match frame["type"].as_str().unwrap() {
            "stream.opened" => dropped = frame["dropped_through_seq"].as_u64().unwrap(),
            "samples.dropped" => dropped = frame["through_seq"].as_u64().unwrap(),
            _ => {
                let seq = frame["seq"].as_u64().unwrap();

                if seq <= last || (seq != last + 1 && last >= dropped) {
                    return false;
                }

                last = seq;
            }
        }
    }

    true
}

fn handshake_status(result: Result<WsClient, tungstenite::Error>) -> u16 {
    match result.err().expect("the handshake must be refused") {
        tungstenite::Error::Http(response) => response.status().as_u16(),
        other => panic!("expected an HTTP refusal, got {other:?}"),
    }
}

#[test]
fn a_client_gets_opened_then_the_replay_then_live_events_then_a_normal_close() {
    let dir = TempDir::new().unwrap();
    let (started, gate) = gated(WsConfig::default());
    let (run_id, _entry) = submit(&started, &dir);

    let mut client = open(&started, run_id);

    let opened = client.json();

    assert_eq!(opened["type"], json!("stream.opened"));
    assert_eq!(opened["run_id"], json!(run_id.to_string()));
    assert_eq!(opened["status"], json!("running"));
    assert_eq!(opened["truncated"], json!(false));
    assert_eq!(opened["dropped_through_seq"], json!(0));

    let first = client.json();

    assert_eq!(first["seq"], json!(1));
    assert_eq!(first["type"], json!("run.accepted"));
    assert_eq!(first["run_id"], json!(run_id.to_string()));

    gate.store(true, Ordering::SeqCst);

    let (rest, code) = client.until_close();

    assert_eq!(code, 1000);
    assert_eq!(rest.last().unwrap()["type"], json!(kind::RUN_COMPLETED));
    assert_eq!(rest.last().unwrap()["data"]["persisted"], json!(true));

    let mut all = vec![opened, first];

    all.extend(rest);

    let numbered = seqs(&all);

    assert_eq!(numbered, (1..=numbered.len() as u64).collect::<Vec<_>>());
    assert!(follows_client_rule(&all, 0));
}

#[test]
fn a_client_that_opens_after_the_run_finished_gets_the_whole_history_and_a_close() {
    let dir = TempDir::new().unwrap();

    let started = start(
        Arc::new(ImmediateExecutor),
        RunRegistry::new(),
        WsConfig::default(),
        None,
    );

    let (run_id, entry) = submit(&started, &dir);

    wait_until_blocking(|| entry.is_terminal(), "the run to finish");

    let mut client = open(&started, run_id);
    let (frames, code) = client.until_close();

    assert_eq!(code, 1000);
    assert_eq!(frames[0]["type"], json!("stream.opened"));
    assert_eq!(frames[0]["status"], json!("completed"));
    assert_eq!(frames[0]["last_seq"], json!(6));
    assert_eq!(seqs(&frames), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(frames.last().unwrap()["type"], json!("run.completed"));
    assert_eq!(started.repository.saved_run_ids(), vec![run_id]);
}

#[test]
fn after_resumes_without_duplicates_and_after_the_last_seq_only_closes() {
    let dir = TempDir::new().unwrap();

    let started = start(
        Arc::new(ImmediateExecutor),
        RunRegistry::new(),
        WsConfig::default(),
        None,
    );

    let (run_id, entry) = submit(&started, &dir);

    wait_until_blocking(|| entry.is_terminal(), "the run to finish");

    let mut resumed = WsClient::open(
        started.server.address,
        &format!("{}?after=3", ws_path(run_id)),
        &[],
    )
    .ok()
    .expect("handshake");

    let (frames, code) = resumed.until_close();

    assert_eq!(code, 1000);
    assert_eq!(seqs(&frames), vec![4, 5, 6]);
    assert!(follows_client_rule(&frames, 3));

    let mut finished = WsClient::open(
        started.server.address,
        &format!("{}?after=6", ws_path(run_id)),
        &[],
    )
    .ok()
    .expect("handshake");

    let (frames, code) = finished.until_close();

    assert_eq!(code, 1000);
    assert_eq!(types(&frames), vec!["stream.opened"]);
}

#[test]
fn an_unknown_run_is_accepted_and_closed_with_4404() {
    let (started, _gate) = gated(WsConfig::default());

    let mut client = open(&started, Uuid::now_v7());

    assert!(matches!(
        client.next(Duration::from_secs(5)),
        Frame::Close(4404)
    ));
}

#[test]
fn a_malformed_run_id_is_refused_before_the_upgrade() {
    let (started, _gate) = gated(WsConfig::default());

    let result = WsClient::open(
        started.server.address,
        "/api/v1/live-runs/not-a-uuid/ws",
        &[],
    );

    assert_eq!(handshake_status(result), 400);
}

#[test]
fn a_malformed_after_is_refused_before_the_upgrade() {
    let (started, _gate) = gated(WsConfig::default());

    let result = WsClient::open(
        started.server.address,
        &format!("{}?after=abc", ws_path(Uuid::now_v7())),
        &[],
    );

    assert_eq!(handshake_status(result), 400);
}

#[test]
fn origins_are_checked_against_the_shared_allow_list() {
    let dir = TempDir::new().unwrap();
    let (started, _gate) = gated(WsConfig::default());
    let (run_id, _entry) = submit(&started, &dir);
    let path = ws_path(run_id);
    let address = started.server.address;

    for refused in [
        "https://evil.example.com",
        "null",
        "http://localhost:5003",
        "https://trycloudflare.com.evil.example.com",
    ] {
        assert_eq!(
            handshake_status(WsClient::open(address, &path, &[refused])),
            403,
            "{refused} must be refused"
        );
    }

    for allowed in [
        "http://localhost:5002",
        "http://127.0.0.1:8092",
        "https://some-name.trycloudflare.com",
        "https://abc-5003.usw3.devtunnels.ms",
    ] {
        let mut client = WsClient::open(address, &path, &[allowed])
            .ok()
            .unwrap_or_else(|| panic!("{allowed} must be allowed"));

        assert_eq!(client.json()["type"], json!("stream.opened"));
    }
}

#[test]
fn a_client_without_an_origin_header_is_allowed_because_it_is_not_a_browser() {
    let dir = TempDir::new().unwrap();
    let (started, _gate) = gated(WsConfig::default());
    let (run_id, _entry) = submit(&started, &dir);

    let mut client = WsClient::open(started.server.address, &ws_path(run_id), &[])
        .ok()
        .expect("a handshake without Origin works");

    assert_eq!(client.json()["type"], json!("stream.opened"));
}

#[test]
fn two_origin_headers_are_refused() {
    let dir = TempDir::new().unwrap();
    let (started, _gate) = gated(WsConfig::default());
    let (run_id, _entry) = submit(&started, &dir);

    let result = WsClient::open(
        started.server.address,
        &ws_path(run_id),
        &["http://localhost:5002", "http://localhost:5002"],
    );

    assert_eq!(handshake_status(result), 403);
}

#[test]
fn a_refused_origin_gets_the_error_shape_of_the_api() {
    let (started, _gate) = gated(WsConfig::default());

    let result = WsClient::open(
        started.server.address,
        &ws_path(Uuid::now_v7()),
        &["https://evil.example.com"],
    );

    match result.err().expect("refused") {
        tungstenite::Error::Http(response) => {
            let body: Value = serde_json::from_slice(response.body().as_ref().unwrap()).unwrap();

            assert_eq!(body["success"], json!(false));
            assert_eq!(body["httpCode"], json!(403));
            assert_eq!(body["code"], json!("FORBIDDEN"));
        }
        other => panic!("expected an HTTP refusal, got {other:?}"),
    }
}

#[test]
fn two_runs_never_leak_events_into_each_others_sockets() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    // The first run finishes at once, the second waits for the gate
    struct FirstFastThenGated {
        calls: std::sync::atomic::AtomicUsize,
        gated: GatedExecutor,
    }

    impl RunExecutor for FirstFastThenGated {
        fn execute(
            &self,
            request: &benchmark_api::features::v1::benchmark::model::BenchmarkRequest,
            target: &std::path::Path,
            env: &benchmark_api::config::env::Env,
            run_id: Uuid,
            ctx: &benchmark_api::features::v1::benchmark::live::RunContext,
            permit: &benchmark_api::features::v1::benchmark::live::RunPermit,
        ) -> Result<
            benchmark_api::features::v1::benchmark::model::BenchmarkResult,
            benchmark_api::features::v1::benchmark::error::BenchmarkError,
        > {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                ImmediateExecutor.execute(request, target, env, run_id, ctx, permit)
            } else {
                self.gated
                    .execute(request, target, env, run_id, ctx, permit)
            }
        }
    }

    let started = start(
        Arc::new(FirstFastThenGated {
            calls: std::sync::atomic::AtomicUsize::new(0),
            gated: GatedExecutor {
                gate: Arc::clone(&gate),
            },
        }),
        RunRegistry::new(),
        WsConfig::default(),
        Some(Arc::clone(&gate)),
    );

    let (first_id, first_entry) = submit(&started, &dir);

    wait_until_blocking(|| first_entry.is_terminal(), "the first run to finish");
    wait_until_blocking(|| started.live.slot().active().is_none(), "the slot");

    let (second_id, _second_entry) = submit(&started, &dir);

    let mut on_first = open(&started, first_id);
    let mut on_second = open(&started, second_id);

    gate.store(true, Ordering::SeqCst);

    let (first_frames, first_code) = on_first.until_close();
    let (second_frames, second_code) = on_second.until_close();

    assert_eq!(first_code, 1000);
    assert_eq!(second_code, 1000);

    for frame in &first_frames {
        assert_eq!(frame["run_id"], json!(first_id.to_string()), "{frame}");
    }

    for frame in &second_frames {
        assert_eq!(frame["run_id"], json!(second_id.to_string()), "{frame}");
    }

    assert_ne!(first_id, second_id);
}

#[test]
fn a_disconnected_client_does_not_affect_the_run_and_its_socket_is_released() {
    let dir = TempDir::new().unwrap();
    let (started, gate) = gated(WsConfig::default());
    let (run_id, entry) = submit(&started, &dir);

    let mut client = open(&started, run_id);

    assert_eq!(client.json()["type"], json!("stream.opened"));

    wait_until_blocking(
        || started.live.sockets().active() == 1,
        "the socket to count",
    );

    drop(client);

    wait_until_blocking(
        || started.live.sockets().active() == 0,
        "the socket to be released",
    );

    assert_eq!(entry.socket_count(), 0);

    gate.store(true, Ordering::SeqCst);

    wait_until_blocking(|| entry.is_terminal(), "the run to finish without a client");
    wait_until_blocking(|| started.live.slot().active().is_none(), "the slot");

    assert_eq!(started.repository.saved_run_ids(), vec![run_id]);
}

#[test]
fn sockets_over_the_per_run_limit_are_closed_with_4429_and_a_place_frees_up() {
    let dir = TempDir::new().unwrap();

    let ws = WsConfig {
        max_sockets_per_run: 2,
        ..WsConfig::default()
    };

    let (started, _gate) = gated(ws);
    let (run_id, _entry) = submit(&started, &dir);

    let mut first = open(&started, run_id);
    let mut second = open(&started, run_id);

    assert_eq!(first.json()["type"], json!("stream.opened"));
    assert_eq!(second.json()["type"], json!("stream.opened"));

    let mut third = open(&started, run_id);

    assert!(matches!(
        third.next(Duration::from_secs(5)),
        Frame::Close(4429)
    ));

    drop(first);

    wait_until_blocking(
        || started.live.sockets().active() == 1,
        "a place to free up",
    );

    let mut fourth = open(&started, run_id);

    assert_eq!(fourth.json()["type"], json!("stream.opened"));
}

#[test]
fn the_total_socket_limit_applies_across_runs() {
    let dir = TempDir::new().unwrap();

    let ws = WsConfig {
        max_sockets_total: 1,
        ..WsConfig::default()
    };

    let (started, _gate) = gated(ws);
    let (run_id, _entry) = submit(&started, &dir);

    let mut first = open(&started, run_id);

    assert_eq!(first.json()["type"], json!("stream.opened"));

    let mut second = open(&started, run_id);

    assert!(matches!(
        second.next(Duration::from_secs(5)),
        Frame::Close(4429)
    ));
}

#[test]
fn a_late_client_of_a_truncated_run_follows_the_gap_rule() {
    let dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let started = start(
        Arc::new(SamplingExecutor {
            samples: 10,
            gate: Arc::clone(&gate),
        }),
        RunRegistry::with_limits(Duration::from_secs(600), 20, 3),
        WsConfig::default(),
        Some(Arc::clone(&gate)),
    );

    let (run_id, entry) = submit(&started, &dir);

    // accepted, stage.started and ten samples
    wait_until_blocking(|| entry.replay(0).last_seq >= 12, "the samples");

    let mut client = open(&started, run_id);

    let opened = client.json();

    assert_eq!(opened["truncated"], json!(true));
    assert_eq!(opened["dropped_through_seq"], json!(9));

    gate.store(true, Ordering::SeqCst);

    let (rest, code) = client.until_close();

    assert_eq!(code, 1000);

    let mut all = vec![opened];

    all.extend(rest);

    let numbered = seqs(&all);

    assert_eq!(&numbered[..5], &[1, 2, 10, 11, 12]);
    assert_eq!(all.last().unwrap()["type"], json!("run.completed"));
    assert!(follows_client_rule(&all, 0));
}

#[test]
fn a_responsive_client_survives_the_heartbeat_and_sees_pings() {
    let dir = TempDir::new().unwrap();

    let ws = WsConfig {
        ping_interval: Duration::from_millis(100),
        pong_timeout: Duration::from_millis(400),
        ..WsConfig::default()
    };

    let (started, _gate) = gated(ws);
    let (run_id, _entry) = submit(&started, &dir);

    let mut client = open(&started, run_id);

    let mut pings = 0;
    let deadline = std::time::Instant::now() + Duration::from_millis(1500);

    while std::time::Instant::now() < deadline {
        if let Frame::Ping = client.next(Duration::from_millis(200)) {
            pings += 1;
        }
    }

    assert!(pings >= 3, "only {pings} pings in 1.5 s");
    assert_eq!(started.live.sockets().active(), 1);
}

#[test]
fn a_client_that_never_answers_pings_is_dropped_and_cleaned_up() {
    let dir = TempDir::new().unwrap();

    let ws = WsConfig {
        ping_interval: Duration::from_millis(100),
        pong_timeout: Duration::from_millis(300),
        ..WsConfig::default()
    };

    let (started, _gate) = gated(ws);
    let (run_id, entry) = submit(&started, &dir);

    // Connected but never read from again, so no pong can be sent
    let _silent = open(&started, run_id);

    wait_until_blocking(
        || started.live.sockets().active() == 1,
        "the socket to count",
    );
    wait_until_blocking(
        || started.live.sockets().active() == 0,
        "the silent socket to be dropped",
    );

    assert_eq!(entry.socket_count(), 0);
}

#[test]
fn an_oversized_client_message_closes_the_socket_but_small_ones_are_ignored() {
    let dir = TempDir::new().unwrap();
    let (started, gate) = gated(WsConfig::default());
    let (run_id, entry) = submit(&started, &dir);

    let mut polite = open(&started, run_id);

    assert_eq!(polite.json()["type"], json!("stream.opened"));

    polite.send_text("hello");

    let mut rude = open(&started, run_id);

    assert_eq!(rude.json()["type"], json!("stream.opened"));

    wait_until_blocking(|| started.live.sockets().active() == 2, "both sockets");

    rude.send_text(&"x".repeat(4096));

    wait_until_blocking(
        || started.live.sockets().active() == 1,
        "the oversized sender to be dropped",
    );

    // The run and the polite socket are unaffected
    gate.store(true, Ordering::SeqCst);

    let (frames, code) = polite.until_close();

    assert_eq!(code, 1000);
    assert_eq!(frames.last().unwrap()["type"], json!("run.completed"));

    wait_until_blocking(|| entry.is_terminal(), "the run to finish");
    thread::sleep(Duration::from_millis(50));
}
