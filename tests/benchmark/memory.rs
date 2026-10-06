use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use benchmark_api::features::v1::benchmark::metrics::memory::{
    self, DEFAULT_SAMPLE_INTERVAL, DEFAULT_STABILIZATION, MemoryConfig, build_memory,
    parse_group_id, parse_vm_rss_bytes,
};
use serde_json::{Value, json};

fn fast_config() -> MemoryConfig {
    MemoryConfig {
        stabilization: Duration::from_millis(0),
        sample_interval: Duration::from_millis(5),
    }
}

// Reader returning values by call number, call numbers start at 1
fn scripted_reader(
    script: fn(u64) -> Result<u64, String>,
) -> (
    impl FnMut() -> Result<u64, String> + Send + 'static,
    Arc<AtomicU64>,
) {
    let calls = Arc::new(AtomicU64::new(0));
    let reader_calls = Arc::clone(&calls);

    let reader = move || {
        let call = reader_calls.fetch_add(1, Ordering::SeqCst) + 1;

        script(call)
    };

    (reader, calls)
}

#[test]
fn serializes_memory_result() {
    let memory = build_memory(52_428_800, 73_400_320);

    let value: Value = serde_json::to_value(memory).unwrap();

    assert_eq!(
        value,
        json!({
            "baseline": { "rss_bytes": 52428800, "rss_size": "50.00 MiB" },
            "peak": { "rss_bytes": 73400320, "rss_size": "70.00 MiB" },
            "delta": { "rss_bytes": 20971520, "rss_size": "20.00 MiB" }
        })
    );
}

#[test]
fn formats_rss_sizes() {
    let memory = build_memory(512, 2048);

    assert_eq!(memory.baseline.rss_size, "512 B");
    assert_eq!(memory.peak.rss_size, "2.00 KiB");
    assert_eq!(memory.delta.rss_size, "1.50 KiB");
}

#[test]
fn delta_is_peak_minus_baseline() {
    let memory = build_memory(1000, 4000);

    assert_eq!(memory.delta.rss_bytes, 3000);
}

#[test]
fn stabilization_and_sampling_interval_keep_their_defaults() {
    assert_eq!(DEFAULT_STABILIZATION, Duration::from_millis(500));
    assert_eq!(DEFAULT_SAMPLE_INTERVAL, Duration::from_millis(100));

    let config = MemoryConfig::default();

    assert_eq!(config.stabilization, Duration::from_millis(500));
    assert_eq!(config.sample_interval, Duration::from_millis(100));
}

#[test]
fn baseline_is_captured_after_stabilization() {
    let started = Instant::now();
    let first_read: Arc<Mutex<Option<Duration>>> = Arc::new(Mutex::new(None));
    let recorder = Arc::clone(&first_read);

    let config = MemoryConfig {
        stabilization: Duration::from_millis(80),
        sample_interval: Duration::from_millis(5),
    };

    let session = memory::begin(&config, move || {
        let mut slot = recorder.lock().unwrap();

        if slot.is_none() {
            *slot = Some(started.elapsed());
        }

        Ok(1000)
    })
    .unwrap();

    session.finish().unwrap();

    let first = first_read.lock().unwrap().unwrap();

    assert!(
        first >= Duration::from_millis(80),
        "baseline read after {first:?}"
    );
}

#[test]
fn baseline_is_the_first_sample() {
    let (reader, _) = scripted_reader(|call| Ok(if call == 1 { 1000 } else { 500 }));

    let session = memory::begin(&fast_config(), reader).unwrap();

    thread::sleep(Duration::from_millis(40));

    let outcome = session.finish().unwrap();

    assert_eq!(outcome.memory.baseline.rss_bytes, 1000);
    assert_eq!(outcome.memory.peak.rss_bytes, 1000);
    assert_eq!(outcome.memory.delta.rss_bytes, 0);
}

#[test]
fn peak_is_the_maximum_sample_seen_during_sampling() {
    let (reader, _) = scripted_reader(|call| {
        Ok(match call {
            1 => 1000,
            3 => 5000,
            _ => 2000,
        })
    });

    let session = memory::begin(&fast_config(), reader).unwrap();

    thread::sleep(Duration::from_millis(80));

    let outcome = session.finish().unwrap();

    assert_eq!(outcome.memory.baseline.rss_bytes, 1000);
    assert_eq!(outcome.memory.peak.rss_bytes, 5000);
    assert_eq!(outcome.memory.delta.rss_bytes, 4000);
}

#[test]
fn sampling_continues_for_the_whole_workload() {
    let (reader, calls) = scripted_reader(|_| Ok(1000));

    let session = memory::begin(&fast_config(), reader).unwrap();

    thread::sleep(Duration::from_millis(150));

    let samples_midway = calls.load(Ordering::SeqCst);

    thread::sleep(Duration::from_millis(150));

    let outcome = session.finish().unwrap();

    assert!(
        samples_midway >= 5,
        "expected repeated sampling, got {samples_midway} reads"
    );
    assert!(
        calls.load(Ordering::SeqCst) > samples_midway,
        "sampling stopped before the workload ended"
    );
    assert!(outcome.samples >= 10);
}

#[test]
fn peak_reflects_a_late_spike_during_the_workload() {
    let (reader, _) = scripted_reader(|call| Ok(if call >= 10 { 9000 } else { 1000 }));

    let session = memory::begin(&fast_config(), reader).unwrap();

    thread::sleep(Duration::from_millis(400));

    let outcome = session.finish().unwrap();

    assert_eq!(outcome.memory.peak.rss_bytes, 9000);
}

#[test]
fn finish_takes_a_final_sample() {
    let config = MemoryConfig {
        stabilization: Duration::from_millis(0),
        sample_interval: Duration::from_secs(60),
    };

    let (reader, calls) = scripted_reader(|call| Ok(if call == 1 { 1000 } else { 7000 }));

    let session = memory::begin(&config, reader).unwrap();
    let outcome = session.finish().unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(outcome.memory.peak.rss_bytes, 7000);
}

#[test]
fn finish_returns_promptly_without_a_fixed_peak_window() {
    let config = MemoryConfig {
        stabilization: Duration::from_millis(0),
        sample_interval: Duration::from_millis(100),
    };

    let (reader, _) = scripted_reader(|_| Ok(1000));

    let started = Instant::now();

    let session = memory::begin(&config, reader).unwrap();

    session.finish().unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "finishing took {:?}",
        started.elapsed()
    );
}

#[test]
fn losing_the_target_keeps_samples_collected_so_far() {
    let (reader, _) = scripted_reader(|call| match call {
        1 => Ok(1000),
        2 => Ok(4000),
        _ => Err("No live processes found in the target process group".to_string()),
    });

    let session = memory::begin(&fast_config(), reader).unwrap();

    thread::sleep(Duration::from_millis(80));

    let outcome = session.finish().unwrap();

    assert!(outcome.target_gone);
    assert_eq!(outcome.memory.baseline.rss_bytes, 1000);
    assert_eq!(outcome.memory.peak.rss_bytes, 4000);
}

#[test]
fn baseline_failure_is_reported() {
    let result = memory::begin(&fast_config(), || Err("gone".to_string()));

    assert!(result.is_err());
}

#[test]
fn parses_group_id_from_stat() {
    let stat = "1234 (bun) S 1 1234 1234 0 -1 4194560";

    assert_eq!(parse_group_id(stat), Some(1234));
}

#[test]
fn parses_group_id_when_comm_has_spaces_and_parentheses() {
    let stat = "1234 (my (odd) app) S 1 777 777 0 -1 4194560";

    assert_eq!(parse_group_id(stat), Some(777));
}

#[test]
fn parses_vm_rss_in_bytes() {
    let status = "Name:\tbun\nVmPeak:\t  900000 kB\nVmRSS:\t   51200 kB\nThreads:\t4\n";

    assert_eq!(parse_vm_rss_bytes(status), Some(52_428_800));
}

#[test]
fn missing_vm_rss_returns_none() {
    let status = "Name:\tkthreadd\nThreads:\t1\n";

    assert_eq!(parse_vm_rss_bytes(status), None);
}
