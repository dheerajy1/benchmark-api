// Baseline versus live measurement comparison, run on request only:
//
//   cargo test --test live overhead -- --ignored --nocapture --test-threads=1
//
// It starts the same bun fixture and the same workload many times and compares the
// results of a run that reports nothing (arm A, the no-op context of the legacy
// endpoint) with a run that is watched (arm B, live samples every 500 ms).
//
// Decision rule, fixed before any measurement:
//
//   1. Runs are paired and the order inside a pair alternates (AB, BA, AB, ...), so a
//      slow drift of the machine cannot favor one arm.
//   2. A second set of pairs runs arm A against arm A. Its largest difference per
//      metric is the noise floor: how much two identical runs differ on this machine.
//   3. Per metric the verdict is "within noise" when the median B-versus-A difference
//      is not larger than the noise floor, and "measurable" otherwise. A measurable
//      difference is reported as the overhead of that metric.
//   4. Nothing is claimed beyond that: "within noise" means this experiment cannot tell
//      the arms apart, not that the cost is zero.
//
// Environment: OVERHEAD_PAIRS (default 10), OVERHEAD_SECONDS (default 5),
// OVERHEAD_COOLDOWN_SECONDS (default 2).

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use benchmark_api::features::v1::benchmark::{
    live::{ProgressSink, RunContext, event::kind},
    model::BenchmarkRequest,
    runtime,
};
use serde_json::Value;
use tempfile::TempDir;

use super::support::{free_port, long_benchmark_body, require_bun, test_env};

#[derive(Default)]
struct CountingSink {
    events: AtomicU64,
    samples: AtomicU64,
}

impl ProgressSink for CountingSink {
    fn emit(&self, event_kind: &'static str, _data: Value) {
        self.events.fetch_add(1, Ordering::SeqCst);

        if event_kind == kind::SAMPLE {
            self.samples.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn wants_samples(&self) -> bool {
        true
    }
}

#[derive(Clone, Copy)]
enum Arm {
    Noop,
    Observed,
}

#[derive(Clone, Copy)]
struct Measured {
    requests_per_second: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    cpu_average_percent: f64,
    cpu_peak_percent: f64,
    memory_peak_bytes: f64,
}

const METRICS: [(&str, fn(&Measured) -> f64); 7] = [
    ("requests_per_second", |m| m.requests_per_second),
    ("p50_ms", |m| m.p50_ms),
    ("p95_ms", |m| m.p95_ms),
    ("p99_ms", |m| m.p99_ms),
    ("cpu_average_percent", |m| m.cpu_average_percent),
    ("cpu_peak_percent", |m| m.cpu_peak_percent),
    ("memory_peak_bytes", |m| m.memory_peak_bytes),
];

fn environment_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

// One complete startup, workload and shutdown on a fresh port
fn run_once(arm: Arm, seconds: u64) -> (Measured, u64) {
    let temp_dir = TempDir::new().unwrap();
    let port = free_port();

    let request: BenchmarkRequest =
        serde_json::from_str(&long_benchmark_body(temp_dir.path(), port, seconds)).unwrap();

    let sink = Arc::new(CountingSink::default());

    let ctx = match arm {
        Arm::Noop => RunContext::noop(),
        Arm::Observed => RunContext::new(Arc::clone(&sink) as Arc<dyn ProgressSink>),
    };

    let metrics = runtime::run_observed(
        Path::new(temp_dir.path()),
        None,
        &request,
        &test_env(),
        &ctx,
    )
    .expect("the run failed");

    let latency = metrics.http_latency.expect("http latency");
    let cpu = metrics.cpu.expect("cpu");
    let memory = metrics.memory.expect("memory");

    assert_eq!(latency.status, "completed", "the target must stay up");

    let measured = Measured {
        requests_per_second: latency.requests_per_second,
        p50_ms: latency.p50_ms.expect("p50"),
        p95_ms: latency.p95_ms.expect("p95"),
        p99_ms: latency.p99_ms.expect("p99"),
        cpu_average_percent: cpu.average_percent,
        cpu_peak_percent: cpu.peak_percent,
        memory_peak_bytes: memory.peak.rss_bytes as f64,
    };

    (measured, sink.samples.load(Ordering::SeqCst))
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();

    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let middle = sorted.len() / 2;

    if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

fn delta_percent(reference: f64, other: f64) -> f64 {
    if reference == 0.0 {
        0.0
    } else {
        (other - reference) / reference * 100.0
    }
}

fn cooldown(seconds: u64) {
    thread::sleep(Duration::from_secs(seconds));
}

#[test]
#[ignore = "takes minutes, starts bun, run explicitly"]
fn observed_versus_noop_workload_overhead() {
    require_bun();

    let pairs = environment_number("OVERHEAD_PAIRS", 10) as usize;
    let seconds = environment_number("OVERHEAD_SECONDS", 5);
    let cooldown_seconds = environment_number("OVERHEAD_COOLDOWN_SECONDS", 2);

    // Arm A against arm B, the order alternates inside the pairs
    let mut a_runs: Vec<Measured> = Vec::new();
    let mut b_runs: Vec<Measured> = Vec::new();
    let mut b_samples: Vec<u64> = Vec::new();

    for pair in 0..pairs {
        let order = if pair % 2 == 0 {
            [Arm::Noop, Arm::Observed]
        } else {
            [Arm::Observed, Arm::Noop]
        };

        for arm in order {
            let (measured, samples) = run_once(arm, seconds);

            match arm {
                Arm::Noop => a_runs.push(measured),
                Arm::Observed => {
                    b_runs.push(measured);
                    b_samples.push(samples);
                }
            }

            cooldown(cooldown_seconds);
        }
    }

    // Arm A against arm A: the noise floor
    let mut noise_pairs: Vec<(Measured, Measured)> = Vec::new();

    for _ in 0..pairs {
        let (first, _) = run_once(Arm::Noop, seconds);

        cooldown(cooldown_seconds);

        let (second, _) = run_once(Arm::Noop, seconds);

        cooldown(cooldown_seconds);

        noise_pairs.push((first, second));
    }

    println!();
    println!(
        "PHASE2 overhead: {pairs} pairs per set, {seconds} s workload, concurrency 2, bun fixture"
    );
    println!(
        "PHASE2 observed samples per run: min {} max {}",
        b_samples.iter().min().unwrap(),
        b_samples.iter().max().unwrap()
    );
    println!(
        "PHASE2 {:<22} {:>14} {:>14} {:>10} {:>10} {:>12}  verdict",
        "metric", "median A", "median B", "B vs A %", "noise %", "B-A range %"
    );

    for (name, read) in METRICS {
        let a_values: Vec<f64> = a_runs.iter().map(read).collect();
        let b_values: Vec<f64> = b_runs.iter().map(read).collect();

        let median_a = median(&a_values);
        let median_b = median(&b_values);
        let median_delta = delta_percent(median_a, median_b);

        let pair_deltas: Vec<f64> = a_runs
            .iter()
            .zip(b_runs.iter())
            .map(|(a, b)| delta_percent(read(a), read(b)))
            .collect();

        let low = pair_deltas.iter().cloned().fold(f64::INFINITY, f64::min);
        let high = pair_deltas
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);

        let noise_floor = noise_pairs
            .iter()
            .map(|(first, second)| delta_percent(read(first), read(second)).abs())
            .fold(0.0, f64::max);

        let verdict = if median_delta.abs() <= noise_floor {
            "within noise".to_string()
        } else {
            format!("measurable: {median_delta:+.2} %")
        };

        println!(
            "PHASE2 {name:<22} {median_a:>14.2} {median_b:>14.2} {median_delta:>+10.2} {noise_floor:>10.2} {:>12}  {verdict}",
            format!("{low:+.1}..{high:+.1}")
        );
    }

    println!();

    assert!(
        b_samples.iter().all(|samples| *samples >= 1),
        "an observed run produced no live sample"
    );
}
