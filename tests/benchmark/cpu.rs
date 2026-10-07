use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use benchmark_api::features::v1::benchmark::metrics::{
    cpu::{
        baseline,
        calc::{MIN_PEAK_INTERVAL, build_cpu, cpu_percent, finalize_peak},
        proc::{ProcessTicks, USER_HZ},
        session,
        tracker::CpuTracker,
    },
    memory::{DEFAULT_SAMPLE_INTERVAL, DEFAULT_STABILIZATION},
};
use serde_json::{Value, json};

type Reading = Result<Vec<ProcessTicks>, String>;

fn ticks(pid: u32, ticks: u64) -> ProcessTicks {
    ProcessTicks { pid, ticks }
}

// Reader returning readings by call number, call numbers start at 1
fn scripted_reader(
    script: fn(u64) -> Reading,
) -> (impl FnMut() -> Reading + Send + 'static, Arc<AtomicU64>) {
    let calls = Arc::new(AtomicU64::new(0));
    let reader_calls = Arc::clone(&calls);

    let reader = move || {
        let call = reader_calls.fetch_add(1, Ordering::SeqCst) + 1;

        script(call)
    };

    (reader, calls)
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

#[test]
fn serializes_cpu_result() {
    let cpu = build_cpu(0.8, 12.4, 28.7);

    let value: Value = serde_json::to_value(cpu).unwrap();

    assert_eq!(
        value,
        json!({
            "baseline_percent": 0.8,
            "average_percent": 12.4,
            "peak_percent": 28.7
        })
    );
}

#[test]
fn cpu_result_has_no_timeseries() {
    let value: Value = serde_json::to_value(build_cpu(1.0, 2.0, 3.0)).unwrap();

    let object = value.as_object().unwrap();

    assert_eq!(object.len(), 3);
    assert!(object.contains_key("baseline_percent"));
    assert!(object.contains_key("average_percent"));
    assert!(object.contains_key("peak_percent"));
}

#[test]
fn rounds_every_value_to_two_decimals() {
    let cpu = build_cpu(1.23456, 2.0, 3.999);

    assert_eq!(cpu.baseline_percent, 1.23);
    assert_eq!(cpu.average_percent, 2.0);
    assert_eq!(cpu.peak_percent, 4.0);
}

#[test]
fn user_hz_and_sampling_defaults_are_unchanged() {
    assert_eq!(USER_HZ, 100);
    assert_eq!(DEFAULT_STABILIZATION, Duration::from_millis(500));
    assert_eq!(DEFAULT_SAMPLE_INTERVAL, Duration::from_millis(100));
    assert!(MIN_PEAK_INTERVAL < DEFAULT_SAMPLE_INTERVAL);
}

#[test]
fn average_is_cpu_time_over_wall_clock_time() {
    // 62 ticks is 0.62 CPU seconds, over 5 seconds that is 12.4 percent
    assert!(close(cpu_percent(62, Duration::from_secs(5)), 12.4));
}

#[test]
fn one_fully_used_core_is_one_hundred_percent() {
    assert_eq!(cpu_percent(100, Duration::from_secs(1)), 100.0);
}

#[test]
fn values_above_one_hundred_percent_are_valid() {
    let percent = cpu_percent(250, Duration::from_secs(1));

    assert_eq!(percent, 250.0);

    let cpu = build_cpu(0.0, percent, 400.0);

    assert_eq!(cpu.average_percent, 250.0);
    assert_eq!(cpu.peak_percent, 400.0);
}

#[test]
fn zero_elapsed_time_gives_zero_percent() {
    assert_eq!(cpu_percent(50, Duration::ZERO), 0.0);
}

#[test]
fn peak_is_never_below_the_average() {
    assert_eq!(finalize_peak(Some(10.0), 20.0), 20.0);
    assert_eq!(finalize_peak(Some(30.0), 20.0), 30.0);
}

#[test]
fn peak_equals_the_average_without_a_valid_sample() {
    assert_eq!(finalize_peak(None, 7.5), 7.5);
}

#[test]
fn tracker_aggregates_the_whole_process_group() {
    let mut tracker = CpuTracker::from_reference(&[ticks(1, 10), ticks(2, 20)]);

    let delta = tracker.observe(&[ticks(1, 15), ticks(2, 30)]);

    assert_eq!(delta, 15);
    assert_eq!(tracker.consumed_ticks(), 15);
}

#[test]
fn tracker_does_not_count_ticks_from_before_the_reference() {
    let mut tracker = CpuTracker::from_reference(&[ticks(1, 100)]);

    assert_eq!(tracker.observe(&[ticks(1, 100)]), 0);
    assert_eq!(tracker.consumed_ticks(), 0);
}

#[test]
fn tracker_counts_all_ticks_of_a_process_that_appears_later() {
    let mut tracker = CpuTracker::from_reference(&[ticks(1, 10)]);

    let delta = tracker.observe(&[ticks(1, 10), ticks(3, 7)]);

    assert_eq!(delta, 7);
    assert_eq!(tracker.consumed_ticks(), 7);
}

#[test]
fn tracker_keeps_the_contribution_of_exited_processes() {
    let mut tracker = CpuTracker::from_reference(&[ticks(1, 0), ticks(2, 0)]);

    assert_eq!(tracker.observe(&[ticks(1, 5), ticks(2, 10)]), 15);

    // Process 2 exited, its ticks stay counted and the total never goes down
    assert_eq!(tracker.observe(&[ticks(1, 8)]), 3);
    assert_eq!(tracker.consumed_ticks(), 18);

    assert_eq!(tracker.observe(&[ticks(1, 8)]), 0);
    assert_eq!(tracker.consumed_ticks(), 18);
}

#[test]
fn tracker_never_goes_negative_when_a_counter_decreases() {
    let mut tracker = CpuTracker::from_reference(&[ticks(1, 50)]);

    assert_eq!(tracker.observe(&[ticks(1, 40)]), 0);
    assert_eq!(tracker.consumed_ticks(), 0);
}

#[test]
fn baseline_is_cpu_time_over_the_window() {
    let (reader, calls) = scripted_reader(|call| Ok(vec![ticks(1, if call == 1 { 0 } else { 5 })]));

    let percent = baseline::measure_baseline(Duration::from_millis(100), reader).unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // 5 ticks is 50 ms of CPU over a window of at least 100 ms
    assert!(percent > 0.0, "baseline was {percent}");
    assert!(percent <= 50.01, "baseline was {percent}");
}

#[test]
fn baseline_waits_for_the_whole_window() {
    let (reader, _) = scripted_reader(|_| Ok(vec![ticks(1, 0)]));
    let started = Instant::now();

    baseline::measure_baseline(Duration::from_millis(120), reader).unwrap();

    assert!(started.elapsed() >= Duration::from_millis(120));
}

#[test]
fn idle_baseline_is_zero() {
    let (reader, _) = scripted_reader(|_| Ok(vec![ticks(1, 42)]));

    let percent = baseline::measure_baseline(Duration::from_millis(20), reader).unwrap();

    assert_eq!(percent, 0.0);
}

#[test]
fn baseline_fails_when_the_first_reading_fails() {
    let (reader, _) = scripted_reader(|_| Err("gone".to_string()));

    assert!(baseline::measure_baseline(Duration::from_millis(10), reader).is_err());
}

#[test]
fn baseline_fails_when_the_group_disappears_during_the_window() {
    let (reader, _) = scripted_reader(|call| {
        if call == 1 {
            Ok(vec![ticks(1, 0)])
        } else {
            Err("gone".to_string())
        }
    });

    assert!(baseline::measure_baseline(Duration::from_millis(10), reader).is_err());
}

#[test]
fn session_start_snapshot_is_read_before_begin_returns() {
    let (reader, calls) = scripted_reader(|_| Ok(vec![ticks(1, 0)]));

    let started = session::begin(0.0, Duration::from_secs(5), reader).unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);

    started.finish().unwrap();
}

#[test]
fn session_start_fails_when_the_group_cannot_be_read() {
    let (reader, _) = scripted_reader(|_| Err("gone".to_string()));

    assert!(session::begin(0.0, Duration::from_millis(50), reader).is_err());
}

#[test]
fn session_reports_average_and_peak_above_one_hundred_percent() {
    let (reader, _) = scripted_reader(|call| {
        let total = if call <= 2 { 0 } else { 600 };

        Ok(vec![ticks(1, total)])
    });

    let started = session::begin(1.5, Duration::from_millis(60), reader).unwrap();

    thread::sleep(Duration::from_millis(300));

    let outcome = started.finish().unwrap();
    let cpu = outcome.cpu.unwrap();

    assert!(!outcome.target_gone);
    assert_eq!(cpu.baseline_percent, 1.5);

    // 600 ticks is 6 CPU seconds, far more than the wall-clock window
    assert!(cpu.average_percent > 100.0, "average was {cpu:?}");
    assert!(cpu.peak_percent >= cpu.average_percent);
    assert!(cpu.peak_percent > 100.0);
}

#[test]
fn session_samples_at_the_configured_interval() {
    let (reader, calls) = scripted_reader(|_| Ok(vec![ticks(1, 0)]));

    let started = session::begin(0.0, Duration::from_millis(50), reader).unwrap();

    thread::sleep(Duration::from_millis(400));

    let outcome = started.finish().unwrap();

    assert!(outcome.samples >= 3, "samples: {}", outcome.samples);
    assert!(outcome.samples <= 12, "samples: {}", outcome.samples);

    // Start snapshot plus one reading per sample
    assert_eq!(calls.load(Ordering::SeqCst), outcome.samples + 1);
}

#[test]
fn peak_equals_average_when_no_interval_was_long_enough() {
    let (reader, _) = scripted_reader(|call| Ok(vec![ticks(1, if call == 1 { 0 } else { 10 })]));

    let started = session::begin(0.0, Duration::from_secs(5), reader).unwrap();
    let outcome = started.finish().unwrap();
    let cpu = outcome.cpu.unwrap();

    assert_eq!(outcome.samples, 1);
    assert_eq!(cpu.peak_percent, cpu.average_percent);
}

#[test]
fn target_disappearance_uses_the_last_successful_reading() {
    let (reader, _) = scripted_reader(|call| match call {
        1 => Ok(vec![ticks(1, 0)]),
        2 => Ok(vec![ticks(1, 30)]),
        _ => Err("gone".to_string()),
    });

    let started = session::begin(0.0, Duration::from_millis(60), reader).unwrap();

    thread::sleep(Duration::from_millis(300));

    let outcome = started.finish().unwrap();

    assert!(outcome.target_gone);
    assert_eq!(outcome.samples, 1);

    let cpu = outcome.cpu.unwrap();

    // 0.3 CPU seconds over about one interval, not over the whole sleep
    assert!(cpu.average_percent > 100.0, "average was {cpu:?}");
}

#[test]
fn target_disappearance_before_the_first_sample_leaves_no_cpu_result() {
    let (reader, _) = scripted_reader(|call| {
        if call == 1 {
            Ok(vec![ticks(1, 0)])
        } else {
            Err("gone".to_string())
        }
    });

    let started = session::begin(0.0, Duration::from_millis(50), reader).unwrap();

    thread::sleep(Duration::from_millis(200));

    let outcome = started.finish().unwrap();

    assert!(outcome.target_gone);
    assert_eq!(outcome.samples, 0);
    assert!(outcome.cpu.is_none());
}

#[test]
fn sampler_stops_after_finish() {
    let (reader, calls) = scripted_reader(|_| Ok(vec![ticks(1, 0)]));

    let started = session::begin(0.0, Duration::from_millis(30), reader).unwrap();

    thread::sleep(Duration::from_millis(100));

    started.finish().unwrap();

    let after_finish = calls.load(Ordering::SeqCst);

    thread::sleep(Duration::from_millis(150));

    assert_eq!(calls.load(Ordering::SeqCst), after_finish);
}
