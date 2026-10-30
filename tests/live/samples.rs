use std::{
    collections::BTreeMap,
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use benchmark_api::features::v1::benchmark::{
    live::{
        LiveGauges, LiveRuns, ProgressSink, RunContext, RunSlot,
        event::{RunEvent, kind},
        gauges::{CpuGauge, RequestCounters, build_sample},
        log::RunStatus,
    },
    metrics::{
        cpu::{proc::ProcessTicks, session},
        memory::{self, MemoryConfig},
    },
    model::BenchmarkRequest,
    request::WorkloadRequest,
    workload::{
        executor::{self, Observer},
        plan::{WorkloadLimit, WorkloadPlan},
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::support::{
    free_port, long_benchmark_body, require_bun, test_env, wait_until_slot_free,
    wait_until_terminal,
};
use crate::fake_repository::InMemoryRepository;

// A watched run that only counts what it receives
#[derive(Default)]
struct CountingSink {
    events: AtomicU64,
}

impl ProgressSink for CountingSink {
    fn emit(&self, _kind: &'static str, _data: Value) {
        self.events.fetch_add(1, Ordering::SeqCst);
    }

    fn wants_samples(&self) -> bool {
        true
    }
}

fn gauges_with_everything() -> LiveGauges {
    let gauges = LiveGauges::default();

    gauges.set_memory(80_000_000, 82_000_000);
    gauges.set_cpu(CpuGauge {
        interval_percent: Some(41.234),
        average_percent: Some(38.9),
        peak_percent: Some(55.0),
    });

    for _ in 0..10 {
        gauges.http.record(true);
    }

    gauges.http.record(false);

    gauges
}

#[test]
fn a_sample_in_request_mode_reports_progress_against_the_known_total() {
    let gauges = gauges_with_everything();

    let sample = build_sample(
        &gauges,
        WorkloadLimit::Requests(100),
        Duration::from_secs(2),
    );

    assert_eq!(
        sample,
        json!({
            "workload_elapsed_ms": 2000,
            "memory": { "rss_bytes": 80_000_000, "peak_rss_bytes": 82_000_000 },
            "cpu": { "interval_percent": 41.23, "average_percent": 38.9, "peak_percent": 55.0 },
            "http": {
                "completed": 11,
                "successful": 10,
                "failed": 1,
                "requests_per_second": 5.5,
                "progress": { "mode": "requests", "done": 11, "total": 100 }
            }
        })
    );
}

#[test]
fn a_sample_in_duration_mode_reports_elapsed_against_the_planned_duration() {
    let gauges = gauges_with_everything();

    let sample = build_sample(
        &gauges,
        WorkloadLimit::Duration(Duration::from_secs(30)),
        Duration::from_millis(2500),
    );

    assert_eq!(
        sample["http"]["progress"],
        json!({ "mode": "duration", "elapsed_ms": 2500, "total_ms": 30000 })
    );
}

#[test]
fn a_sample_has_no_values_that_were_not_measured() {
    let gauges = LiveGauges::default();

    let sample = build_sample(
        &gauges,
        WorkloadLimit::Requests(10),
        Duration::from_millis(500),
    );

    assert_eq!(sample["memory"], Value::Null);
    assert_eq!(sample["cpu"], Value::Null);
    assert_eq!(sample["http"]["completed"], json!(0));
    assert_eq!(sample["http"]["requests_per_second"], json!(0.0));
}

#[test]
fn a_cpu_value_that_is_not_measurable_yet_is_null() {
    let gauges = LiveGauges::default();

    gauges.set_cpu(CpuGauge {
        interval_percent: None,
        average_percent: Some(12.0),
        peak_percent: None,
    });

    let sample = build_sample(&gauges, WorkloadLimit::Requests(10), Duration::from_secs(1));

    assert_eq!(
        sample["cpu"],
        json!({ "interval_percent": null, "average_percent": 12.0, "peak_percent": null })
    );
}

#[test]
fn a_sample_never_contains_latency_statistics() {
    let sample = build_sample(
        &gauges_with_everything(),
        WorkloadLimit::Requests(100),
        Duration::from_secs(2),
    );

    let http_keys: Vec<&str> = sample["http"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();

    for forbidden in ["p50_ms", "p95_ms", "p99_ms", "min_ms", "mean_ms", "max_ms"] {
        assert!(!http_keys.contains(&forbidden), "{forbidden} in a sample");
    }

    let mut keys = http_keys.clone();

    keys.sort_unstable();

    assert_eq!(
        keys,
        vec![
            "completed",
            "failed",
            "progress",
            "requests_per_second",
            "successful"
        ]
    );

    let mut top: Vec<&str> = sample
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();

    top.sort_unstable();

    assert_eq!(top, vec!["cpu", "http", "memory", "workload_elapsed_ms"]);
}

#[test]
fn a_snapshot_never_shows_more_successful_than_completed_requests() {
    let counters = Arc::new(RequestCounters::default());

    let writers: Vec<_> = (0..4)
        .map(|_| {
            let counters = Arc::clone(&counters);

            thread::spawn(move || {
                for index in 0..50_000u32 {
                    counters.record(index % 3 != 0);
                }
            })
        })
        .collect();

    for _ in 0..20_000 {
        let (completed, successful) = counters.snapshot();

        assert!(successful <= completed, "{successful} > {completed}");
    }

    for writer in writers {
        writer.join().unwrap();
    }

    let (completed, successful) = counters.snapshot();

    assert_eq!(completed, 200_000);
    assert_eq!(successful, 4 * (50_000 - 16_667));
}

#[test]
fn the_memory_sampler_publishes_its_readings_to_the_gauges() {
    let gauges = Arc::new(LiveGauges::default());
    let readings = Arc::new(AtomicU64::new(1_000));
    let reader_readings = Arc::clone(&readings);

    let config = MemoryConfig {
        stabilization: Duration::ZERO,
        sample_interval: Duration::from_millis(10),
    };

    let session = memory::begin_observed(
        &config,
        move || Ok(reader_readings.fetch_add(100, Ordering::SeqCst)),
        Some(Arc::clone(&gauges)),
    )
    .unwrap();

    let first = gauges.memory().expect("the baseline is published at once");

    assert!(first.rss_bytes >= 1_000);
    assert!(first.peak_rss_bytes >= first.rss_bytes);

    thread::sleep(Duration::from_millis(150));

    let live = gauges.memory().unwrap();

    assert!(live.rss_bytes > 1_000);
    assert_eq!(live.peak_rss_bytes, live.rss_bytes);

    let outcome = session.finish().unwrap();

    assert_eq!(outcome.memory.baseline.rss_bytes, 1_000);
    assert!(outcome.memory.peak.rss_bytes >= live.peak_rss_bytes);
}

#[test]
fn the_cpu_sampler_publishes_its_samples_to_the_gauges() {
    let gauges = Arc::new(LiveGauges::default());
    let calls = Arc::new(AtomicU64::new(0));
    let reader_calls = Arc::clone(&calls);

    let session = session::begin_observed(
        0.0,
        Duration::from_millis(60),
        move || {
            let call = reader_calls.fetch_add(1, Ordering::SeqCst);

            Ok(vec![ProcessTicks {
                pid: 1,
                ticks: call * 3,
            }])
        },
        Some(Arc::clone(&gauges)),
    )
    .unwrap();

    thread::sleep(Duration::from_millis(400));

    let live = gauges.cpu().expect("CPU samples were taken");

    assert!(live.average_percent.is_some_and(|average| average > 0.0));
    assert!(live.interval_percent.is_some_and(|interval| interval > 0.0));
    assert!(live.peak_percent.is_some_and(|peak| peak > 0.0));

    let outcome = session.finish().unwrap();
    let cpu = outcome.cpu.expect("a CPU result");

    assert!(cpu.peak_percent >= live.peak_percent.unwrap() - 0.01);
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();

    listener.local_addr().unwrap().port()
}

fn duration_plan(port: u16, seconds: u64) -> WorkloadPlan {
    let request = WorkloadRequest {
        method: "GET".to_string(),
        path: "/".to_string(),
        headers: BTreeMap::new(),
        requests: None,
        duration_seconds: Some(seconds),
        concurrency: 2,
    };

    WorkloadPlan::new(&request, "127.0.0.1", port, Duration::from_secs(2)).unwrap()
}

#[test]
fn the_executor_ticks_on_the_interval_and_counts_every_request() {
    let plan = duration_plan(closed_port(), 1);
    let counters = RequestCounters::default();

    let mut ticks: Vec<Duration> = Vec::new();
    let mut on_tick = |elapsed: Duration| ticks.push(elapsed);

    let outcome = executor::execute_observed(
        &plan,
        || true,
        Some(Observer {
            counters: &counters,
            interval: Duration::from_millis(100),
            on_tick: &mut on_tick,
        }),
    )
    .unwrap();

    let (completed, successful) = counters.snapshot();

    // Nothing listens on the port, so every request fails, and every one is counted
    assert!(completed > 0);
    assert_eq!(completed, outcome.observations.len() as u64);
    assert_eq!(successful, 0);

    assert!(ticks.len() >= 5, "only {} ticks in 1 s", ticks.len());
    assert!(ticks.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(ticks[0] >= Duration::from_millis(100));
    assert!(*ticks.last().unwrap() <= outcome.elapsed);
}

#[test]
fn the_executor_without_an_observer_still_returns_the_same_outcome_shape() {
    let plan = duration_plan(closed_port(), 1);

    let outcome = executor::execute(&plan, || true).unwrap();

    assert!(!outcome.observations.is_empty());
    assert!(outcome.observations.iter().all(|request| !request.success));
}

#[test]
fn only_a_watched_run_asks_for_samples() {
    assert!(!RunContext::noop().samples_enabled());

    let counting = CountingSink::default();

    assert!(counting.wants_samples());
    assert!(RunContext::new(Arc::new(counting)).samples_enabled());
}

// A real run on a real bun target: the samples of the workload are consistent with
// each other and with the final result
#[tokio::test(flavor = "multi_thread")]
async fn a_real_workload_emits_consistent_samples_between_its_stage_events() {
    require_bun();

    let temp_dir = TempDir::new().unwrap();
    let port = free_port();
    let repository = Arc::new(InMemoryRepository::default());
    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());

    let request: BenchmarkRequest =
        serde_json::from_str(&long_benchmark_body(temp_dir.path(), port, 3)).unwrap();

    let accepted = live.submit(request).expect("accepted");

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    let replay = accepted.entry.replay(0);

    assert_eq!(
        replay.status,
        RunStatus::Completed,
        "{:?}",
        replay.events.last()
    );
    assert!(!replay.truncated());

    let position = |wanted_kind: &str, stage: &str| {
        replay
            .events
            .iter()
            .position(|event| event.kind == wanted_kind && event.data["stage"] == json!(stage))
            .unwrap_or_else(|| panic!("no {wanted_kind} for {stage}"))
    };

    let baseline_measured = replay
        .events
        .iter()
        .position(|event| event.kind == kind::BASELINE_MEASURED)
        .expect("baseline.measured");
    let workload_started = position(kind::STAGE_STARTED, "workload");
    let workload_completed = position(kind::STAGE_COMPLETED, "workload");

    let samples: Vec<(usize, &RunEvent)> = replay
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.kind == kind::SAMPLE)
        .collect();

    // A 3 s workload at a 500 ms interval gives about 5 samples
    assert!(samples.len() >= 3, "only {} samples", samples.len());
    assert!(samples.len() <= 7, "{} samples", samples.len());

    for (index, _) in &samples {
        assert!(*index > workload_started && *index < workload_completed);
        assert!(*index > baseline_measured);
    }

    let mut last_elapsed = 0u64;
    let mut last_completed = 0u64;
    let mut last_t_ms = 0u64;

    for (_, event) in &samples {
        let data = &event.data;
        let elapsed = data["workload_elapsed_ms"].as_u64().unwrap();
        let completed = data["http"]["completed"].as_u64().unwrap();
        let successful = data["http"]["successful"].as_u64().unwrap();
        let failed = data["http"]["failed"].as_u64().unwrap();

        assert!(
            elapsed >= last_elapsed && elapsed >= 450,
            "elapsed {elapsed}"
        );
        assert!(completed >= last_completed);
        assert!(event.t_ms >= last_t_ms);
        assert_eq!(successful + failed, completed);
        assert_eq!(
            data["http"]["progress"]["mode"],
            json!("duration"),
            "{data}"
        );
        assert_eq!(data["http"]["progress"]["total_ms"], json!(3000));

        let memory: MemoryGaugeJson = MemoryGaugeJson::from(&data["memory"]);

        assert!(memory.rss > 0);
        assert!(memory.peak >= memory.rss);

        last_elapsed = elapsed;
        last_completed = completed;
        last_t_ms = event.t_ms;
    }

    assert!(last_completed > 0);

    let result = &replay.events.last().unwrap().data["result"]["metrics"];
    let total_requests = result["http_latency"]["total_requests"].as_u64().unwrap();
    let final_peak = result["memory"]["peak"]["rss_bytes"].as_u64().unwrap();

    // The last sample is taken before the workload ends, so it never exceeds the final totals
    assert!(last_completed <= total_requests);

    let last_peak = samples.last().unwrap().1.data["memory"]["peak_rss_bytes"]
        .as_u64()
        .unwrap();

    assert!(last_peak <= final_peak);

    // The CPU average is only reported once it can be computed
    assert!(
        samples
            .iter()
            .any(|(_, event)| event.data["cpu"] != Value::Null)
    );

    assert_eq!(repository.saved_run_ids(), vec![accepted.run_id]);
}

struct MemoryGaugeJson {
    rss: u64,
    peak: u64,
}

impl From<&Value> for MemoryGaugeJson {
    fn from(value: &Value) -> Self {
        Self {
            rss: value["rss_bytes"].as_u64().unwrap(),
            peak: value["peak_rss_bytes"].as_u64().unwrap(),
        }
    }
}
