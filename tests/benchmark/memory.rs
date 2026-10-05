use std::time::Duration;

use benchmark_api::features::v1::benchmark::metrics::memory::{
    MemoryConfig, build_memory, measure_with_sampler, parse_group_id, parse_vm_rss_bytes,
};
use serde_json::{Value, json};

fn fast_config() -> MemoryConfig {
    MemoryConfig {
        stabilization: Duration::from_millis(0),
        sample_interval: Duration::from_millis(5),
        sample_window: Duration::from_millis(60),
    }
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
fn baseline_is_first_sample() {
    let mut calls = 0u64;

    let memory = measure_with_sampler(&fast_config(), || {
        calls += 1;
        Ok(if calls == 1 { 1000 } else { 500 })
    })
    .unwrap();

    assert_eq!(memory.baseline.rss_bytes, 1000);
}

#[test]
fn peak_is_maximum_sample() {
    let mut calls = 0u64;

    let memory = measure_with_sampler(&fast_config(), || {
        calls += 1;
        Ok(match calls {
            1 => 1000,
            3 => 5000,
            _ => 2000,
        })
    })
    .unwrap();

    assert_eq!(memory.baseline.rss_bytes, 1000);
    assert_eq!(memory.peak.rss_bytes, 5000);
    assert_eq!(memory.delta.rss_bytes, 4000);
}

#[test]
fn peak_is_never_below_baseline() {
    let mut calls = 0u64;

    let memory = measure_with_sampler(&fast_config(), || {
        calls += 1;
        Ok(if calls == 1 { 3000 } else { 1000 })
    })
    .unwrap();

    assert_eq!(memory.peak.rss_bytes, 3000);
    assert_eq!(memory.delta.rss_bytes, 0);
}

#[test]
fn samples_repeatedly_during_window() {
    let mut calls = 0u64;

    measure_with_sampler(&fast_config(), || {
        calls += 1;
        Ok(1000)
    })
    .unwrap();

    assert!(
        calls >= 3,
        "expected repeated sampling, got {calls} samples"
    );
}

#[test]
fn sampler_failure_returns_error() {
    let mut calls = 0u64;

    let result = measure_with_sampler(&fast_config(), || {
        calls += 1;

        if calls == 3 {
            Err("Target process exited during memory measurement".to_string())
        } else {
            Ok(1000)
        }
    });

    assert_eq!(
        result.err().unwrap(),
        "Target process exited during memory measurement"
    );
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
