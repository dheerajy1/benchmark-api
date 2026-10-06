use std::time::Duration;

use benchmark_api::features::v1::benchmark::{
    metrics::http_latency::{self, percentile, round_two_decimals},
    workload::observation::RequestObservation,
};
use serde_json::{Value, json};

fn observation(millis: u64, success: bool) -> RequestObservation {
    RequestObservation {
        duration: Duration::from_millis(millis),
        success,
    }
}

fn hundred_observations() -> Vec<RequestObservation> {
    (1..=100).map(|millis| observation(millis, true)).collect()
}

#[test]
fn computes_min_mean_percentiles_and_max() {
    let result = http_latency::summarize(&hundred_observations(), Duration::from_secs(1), false);

    assert_eq!(result.min_ms, Some(1.0));
    assert_eq!(result.mean_ms, Some(50.5));
    assert_eq!(result.p50_ms, Some(50.0));
    assert_eq!(result.p95_ms, Some(95.0));
    assert_eq!(result.p99_ms, Some(99.0));
    assert_eq!(result.max_ms, Some(100.0));
}

#[test]
fn percentiles_use_nearest_rank() {
    let sorted: Vec<Duration> = (1..=10).map(Duration::from_millis).collect();

    assert_eq!(percentile(&sorted, 50), Some(Duration::from_millis(5)));
    assert_eq!(percentile(&sorted, 95), Some(Duration::from_millis(10)));
    assert_eq!(percentile(&sorted, 99), Some(Duration::from_millis(10)));
    assert_eq!(percentile(&sorted, 10), Some(Duration::from_millis(1)));
}

#[test]
fn percentile_of_empty_input_is_none() {
    assert_eq!(percentile(&[], 50), None);
}

#[test]
fn single_observation_fills_every_statistic() {
    let result = http_latency::summarize(&[observation(7, true)], Duration::from_secs(1), false);

    assert_eq!(result.min_ms, Some(7.0));
    assert_eq!(result.mean_ms, Some(7.0));
    assert_eq!(result.p50_ms, Some(7.0));
    assert_eq!(result.p95_ms, Some(7.0));
    assert_eq!(result.p99_ms, Some(7.0));
    assert_eq!(result.max_ms, Some(7.0));
}

#[test]
fn unsorted_observations_are_handled() {
    let observations = [
        observation(30, true),
        observation(10, true),
        observation(20, true),
    ];

    let result = http_latency::summarize(&observations, Duration::from_secs(1), false);

    assert_eq!(result.min_ms, Some(10.0));
    assert_eq!(result.p50_ms, Some(20.0));
    assert_eq!(result.max_ms, Some(30.0));
}

#[test]
fn counts_successful_and_failed_requests() {
    let observations = [
        observation(1, true),
        observation(2, false),
        observation(3, true),
        observation(4, false),
        observation(5, false),
    ];

    let result = http_latency::summarize(&observations, Duration::from_secs(1), false);

    assert_eq!(result.total_requests, 5);
    assert_eq!(result.successful_requests, 2);
    assert_eq!(result.failed_requests, 3);
}

#[test]
fn failed_requests_contribute_their_duration() {
    let observations = [observation(5, true), observation(900, false)];

    let result = http_latency::summarize(&observations, Duration::from_secs(1), false);

    assert_eq!(result.max_ms, Some(900.0));
    assert_eq!(result.mean_ms, Some(452.5));
}

#[test]
fn duration_and_throughput_come_from_workload_elapsed_time() {
    let result = http_latency::summarize(&hundred_observations(), Duration::from_secs(2), false);

    assert_eq!(result.duration_ms, 2000);
    assert_eq!(result.requests_per_second, 50.0);
}

#[test]
fn throughput_is_rounded_to_two_decimals() {
    let observations: Vec<RequestObservation> = (0..1000).map(|_| observation(1, true)).collect();

    let result = http_latency::summarize(&observations, Duration::from_millis(8234), false);

    assert_eq!(result.duration_ms, 8234);
    assert_eq!(result.requests_per_second, 121.45);
}

#[test]
fn milliseconds_are_rounded_to_two_decimals() {
    let observations = [RequestObservation {
        duration: Duration::from_nanos(1_234_560),
        success: true,
    }];

    let result = http_latency::summarize(&observations, Duration::from_secs(1), false);

    assert_eq!(result.min_ms, Some(1.23));
    assert_eq!(round_two_decimals(2.345_6), 2.35);
}

#[test]
fn empty_observations_have_no_statistics() {
    let result = http_latency::summarize(&[], Duration::ZERO, true);

    assert_eq!(result.total_requests, 0);
    assert_eq!(result.successful_requests, 0);
    assert_eq!(result.failed_requests, 0);
    assert_eq!(result.requests_per_second, 0.0);
    assert!(result.min_ms.is_none());
    assert!(result.mean_ms.is_none());
    assert!(result.p50_ms.is_none());
    assert!(result.p95_ms.is_none());
    assert!(result.p99_ms.is_none());
    assert!(result.max_ms.is_none());
}

#[test]
fn status_reflects_whether_the_target_exited() {
    let completed = http_latency::summarize(&hundred_observations(), Duration::from_secs(1), false);
    let exited = http_latency::summarize(&hundred_observations(), Duration::from_secs(1), true);

    assert_eq!(completed.status, "completed");
    assert_eq!(exited.status, "target_exited");
}

#[test]
fn serializes_the_result_contract() {
    let result = http_latency::summarize(&[observation(4, true)], Duration::from_secs(1), false);

    let value: Value = serde_json::to_value(result).unwrap();

    assert_eq!(
        value,
        json!({
            "status": "completed",
            "total_requests": 1,
            "successful_requests": 1,
            "failed_requests": 0,
            "duration_ms": 1000,
            "requests_per_second": 1.0,
            "min_ms": 4.0,
            "mean_ms": 4.0,
            "p50_ms": 4.0,
            "p95_ms": 4.0,
            "p99_ms": 4.0,
            "max_ms": 4.0
        })
    );
}
