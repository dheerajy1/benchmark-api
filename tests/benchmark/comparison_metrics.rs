use benchmark_api::{
    features::v1::comparison::{compare::compare, metric},
    persistence::{postgres::mapping::run_created_at, stored_run::StoredRun},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::comparison_sample::{result, skipped};
use benchmark_api::features::v1::benchmark::model::BenchmarkResult;

fn stored(result: BenchmarkResult) -> StoredRun {
    StoredRun {
        created_at: run_created_at(result.run_id).unwrap(),
        result,
    }
}

fn run() -> BenchmarkResult {
    result(Uuid::now_v7())
}

// Compares two runs and returns the response as JSON, exactly what a client receives
fn compared(baseline: BenchmarkResult, comparison: BenchmarkResult) -> Value {
    serde_json::to_value(compare(&stored(baseline), &stored(comparison))).unwrap()
}

#[test]
fn identical_runs_have_no_difference() {
    let value = compared(run(), run());
    let metrics = &value["metrics"];

    assert_eq!(
        metrics["build_time"]["duration_ms"]["absolute_difference"],
        json!(0)
    );
    assert_eq!(
        metrics["build_time"]["duration_ms"]["percentage_change"],
        json!(0.0)
    );
    assert_eq!(
        metrics["cpu"]["peak_percent"]["absolute_difference"],
        json!(0.0)
    );
    assert_eq!(
        metrics["http_latency"]["p99_ms"]["percentage_change"],
        json!(0.0)
    );
}

#[test]
fn baseline_and_comparison_are_identified() {
    let baseline = run();
    let comparison = run();

    let value = compared(baseline.clone(), comparison.clone());

    assert_eq!(
        value["baseline"]["run_id"],
        json!(baseline.run_id.to_string())
    );
    assert_eq!(
        value["comparison"]["run_id"],
        json!(comparison.run_id.to_string())
    );
    assert_eq!(
        value["baseline"]["target"],
        json!({ "path": "/projects/api" })
    );
    assert_eq!(value["baseline"]["runtime"], json!("bun"));
    assert!(
        value["baseline"]["created_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
}

#[test]
fn the_order_of_the_runs_matters() {
    let first = run();
    let mut second = run();

    second.metrics.build_time.duration_ms = Some(1500);

    let forward = compared(first.clone(), second.clone());
    let backward = compared(second, first);

    let forward = &forward["metrics"]["build_time"]["duration_ms"];
    let backward = &backward["metrics"]["build_time"]["duration_ms"];

    assert_eq!(forward["baseline_value"], json!(1000));
    assert_eq!(forward["comparison_value"], json!(1500));
    assert_eq!(forward["absolute_difference"], json!(500));
    assert_eq!(forward["percentage_change"], json!(50.0));

    assert_eq!(backward["baseline_value"], json!(1500));
    assert_eq!(backward["comparison_value"], json!(1000));
    assert_eq!(backward["absolute_difference"], json!(-500));
    assert_eq!(backward["percentage_change"], json!(-33.33));
}

#[test]
fn build_time_is_compared() {
    let mut comparison = run();

    comparison.metrics.build_time.duration_ms = Some(750);

    let value = compared(run(), comparison);
    let build = &value["metrics"]["build_time"];

    assert_eq!(build["baseline_status"], json!("success"));
    assert_eq!(build["comparison_status"], json!("success"));
    assert_eq!(build["duration_ms"]["baseline_value"], json!(1000));
    assert_eq!(build["duration_ms"]["comparison_value"], json!(750));
    assert_eq!(build["duration_ms"]["absolute_difference"], json!(-250));
    assert_eq!(build["duration_ms"]["percentage_change"], json!(-25.0));
    assert_eq!(build["duration_ms"]["direction"], json!("lower_is_better"));
}

#[test]
fn startup_time_is_compared() {
    let mut comparison = run();

    comparison.metrics.startup_time.duration_ms = Some(600);

    let value = compared(run(), comparison);
    let startup = &value["metrics"]["startup_time"]["duration_ms"];

    assert_eq!(startup["baseline_value"], json!(500));
    assert_eq!(startup["comparison_value"], json!(600));
    assert_eq!(startup["absolute_difference"], json!(100));
    assert_eq!(startup["percentage_change"], json!(20.0));
    assert_eq!(startup["direction"], json!("lower_is_better"));
}

#[test]
fn memory_is_compared() {
    let mut comparison = run();
    let memory = comparison.metrics.memory.as_mut().unwrap();

    memory.baseline.rss_bytes = 150;
    memory.peak.rss_bytes = 100;
    memory.delta.rss_bytes = 100;

    let value = compared(run(), comparison);
    let memory = &value["metrics"]["memory"];

    assert_eq!(
        memory["baseline_rss_bytes"]["absolute_difference"],
        json!(50)
    );
    assert_eq!(
        memory["baseline_rss_bytes"]["percentage_change"],
        json!(50.0)
    );
    assert_eq!(memory["peak_rss_bytes"]["absolute_difference"], json!(-100));
    assert_eq!(memory["peak_rss_bytes"]["percentage_change"], json!(-50.0));
    assert_eq!(memory["delta_rss_bytes"]["absolute_difference"], json!(0));

    for field in ["baseline_rss_bytes", "peak_rss_bytes", "delta_rss_bytes"] {
        assert_eq!(
            memory[field]["direction"],
            json!("lower_is_better"),
            "{field}"
        );
    }
}

#[test]
fn cpu_is_compared() {
    let mut comparison = run();
    let cpu = comparison.metrics.cpu.as_mut().unwrap();

    cpu.baseline_percent = 15.0;
    cpu.average_percent = 10.0;
    cpu.peak_percent = 60.0;

    let value = compared(run(), comparison);
    let cpu = &value["metrics"]["cpu"];

    assert_eq!(cpu["baseline_percent"]["absolute_difference"], json!(5.0));
    assert_eq!(cpu["baseline_percent"]["percentage_change"], json!(50.0));
    assert_eq!(cpu["average_percent"]["absolute_difference"], json!(-10.0));
    assert_eq!(cpu["average_percent"]["percentage_change"], json!(-50.0));
    assert_eq!(cpu["peak_percent"]["absolute_difference"], json!(20.0));
    assert_eq!(cpu["peak_percent"]["percentage_change"], json!(50.0));

    for field in ["baseline_percent", "average_percent", "peak_percent"] {
        assert_eq!(cpu[field]["direction"], json!("lower_is_better"), "{field}");
    }
}

#[test]
fn http_throughput_is_compared_and_higher_is_better() {
    let mut comparison = run();

    comparison
        .metrics
        .http_latency
        .as_mut()
        .unwrap()
        .requests_per_second = 150.0;

    let value = compared(run(), comparison);
    let throughput = &value["metrics"]["http_latency"]["requests_per_second"];

    assert_eq!(throughput["baseline_value"], json!(100.0));
    assert_eq!(throughput["comparison_value"], json!(150.0));
    assert_eq!(throughput["absolute_difference"], json!(50.0));
    assert_eq!(throughput["percentage_change"], json!(50.0));
    assert_eq!(throughput["direction"], json!("higher_is_better"));
}

#[test]
fn http_latency_percentiles_are_compared_and_lower_is_better() {
    let mut comparison = run();
    let latency = comparison.metrics.http_latency.as_mut().unwrap();

    latency.p50_ms = Some(5.0);
    latency.p95_ms = Some(30.0);
    latency.p99_ms = Some(40.0);

    let value = compared(run(), comparison);
    let http = &value["metrics"]["http_latency"];

    assert_eq!(http["p50_ms"]["absolute_difference"], json!(-5.0));
    assert_eq!(http["p50_ms"]["percentage_change"], json!(-50.0));
    assert_eq!(http["p95_ms"]["absolute_difference"], json!(10.0));
    assert_eq!(http["p95_ms"]["percentage_change"], json!(50.0));
    assert_eq!(http["p99_ms"]["absolute_difference"], json!(0.0));

    for field in ["p50_ms", "p95_ms", "p99_ms"] {
        assert_eq!(
            http[field]["direction"],
            json!("lower_is_better"),
            "{field}"
        );
    }

    // Only the four chosen HTTP metrics are compared, not every stored field
    let keys: Vec<&String> = http.as_object().unwrap().keys().collect();

    assert!(
        !keys
            .iter()
            .any(|key| key.as_str() == "min_ms" || key.as_str() == "max_ms")
    );
}

#[test]
fn artifact_source_is_compared() {
    let mut comparison = run();

    comparison.metrics.artifact_size.source.bytes = 1500;
    comparison.metrics.artifact_size.source.file_count = 5;

    let value = compared(run(), comparison);
    let source = &value["metrics"]["artifact_size"]["source"];

    assert_eq!(source["bytes"]["absolute_difference"], json!(500));
    assert_eq!(source["bytes"]["percentage_change"], json!(50.0));
    assert_eq!(source["file_count"]["absolute_difference"], json!(-5));
    assert_eq!(source["file_count"]["percentage_change"], json!(-50.0));
    assert_eq!(source["bytes"]["direction"], json!("lower_is_better"));
    assert_eq!(source["file_count"]["direction"], json!("lower_is_better"));
}

#[test]
fn artifact_build_is_compared() {
    let mut comparison = run();

    comparison.metrics.artifact_size.build.bytes = 1000;
    comparison.metrics.artifact_size.build.file_count = 30;

    let value = compared(run(), comparison);
    let build = &value["metrics"]["artifact_size"]["build"];

    assert_eq!(build["bytes"]["absolute_difference"], json!(-1000));
    assert_eq!(build["bytes"]["percentage_change"], json!(-50.0));
    assert_eq!(build["file_count"]["absolute_difference"], json!(10));
    assert_eq!(build["file_count"]["percentage_change"], json!(50.0));
}

#[test]
fn artifact_excluded_is_compared() {
    let mut comparison = run();

    comparison.metrics.artifact_size.excluded.bytes = 6000;
    comparison.metrics.artifact_size.excluded.file_count = 30;

    let value = compared(run(), comparison);
    let excluded = &value["metrics"]["artifact_size"]["excluded"];

    assert_eq!(excluded["bytes"]["absolute_difference"], json!(3000));
    assert_eq!(excluded["bytes"]["percentage_change"], json!(100.0));
    assert_eq!(excluded["file_count"]["absolute_difference"], json!(0));
    assert_eq!(excluded["file_count"]["percentage_change"], json!(0.0));
}

#[test]
fn a_missing_metric_produces_null_values_not_zero() {
    let mut comparison = run();

    comparison.metrics.memory = None;

    let value = compared(run(), comparison);
    let peak = &value["metrics"]["memory"]["peak_rss_bytes"];

    assert_eq!(peak["baseline_value"], json!(200));
    assert_eq!(peak["comparison_value"], json!(null));
    assert_eq!(peak["absolute_difference"], json!(null));
    assert_eq!(peak["percentage_change"], json!(null));
    assert_eq!(peak["direction"], json!("lower_is_better"));
}

#[test]
fn a_missing_baseline_metric_is_null_on_the_baseline_side() {
    let mut baseline = run();

    baseline.metrics.cpu = None;

    let value = compared(baseline, run());
    let average = &value["metrics"]["cpu"]["average_percent"];

    assert_eq!(average["baseline_value"], json!(null));
    assert_eq!(average["comparison_value"], json!(20.0));
    assert_eq!(average["absolute_difference"], json!(null));
    assert_eq!(average["percentage_change"], json!(null));
}

#[test]
fn a_zero_baseline_has_no_percentage_change() {
    let mut baseline = run();
    let mut comparison = run();

    baseline.metrics.cpu.as_mut().unwrap().baseline_percent = 0.0;
    comparison.metrics.cpu.as_mut().unwrap().baseline_percent = 5.0;

    baseline.metrics.artifact_size.excluded.file_count = 0;
    comparison.metrics.artifact_size.excluded.file_count = 4;

    let value = compared(baseline, comparison);

    let cpu = &value["metrics"]["cpu"]["baseline_percent"];

    assert_eq!(cpu["absolute_difference"], json!(5.0));
    assert_eq!(cpu["percentage_change"], json!(null));

    let files = &value["metrics"]["artifact_size"]["excluded"]["file_count"];

    assert_eq!(files["absolute_difference"], json!(4));
    assert_eq!(files["percentage_change"], json!(null));
}

#[test]
fn percentage_changes_are_signed_and_rounded_to_two_decimals() {
    let increase = metric::integer(Some(300), Some(400), metric::Direction::LowerIsBetter);
    let decrease = metric::integer(Some(400), Some(300), metric::Direction::LowerIsBetter);
    let fractional = metric::decimal(Some(3.0), Some(4.0), metric::Direction::LowerIsBetter);

    assert_eq!(increase.percentage_change, Some(33.33));
    assert_eq!(decrease.percentage_change, Some(-25.0));
    assert_eq!(fractional.percentage_change, Some(33.33));
    assert_eq!(fractional.absolute_difference, Some(1.0));
}

#[test]
fn a_failed_build_or_startup_is_not_a_comparable_duration() {
    let mut comparison = run();

    comparison.metrics.build_time.status = "failed".to_string();
    comparison.metrics.build_time.duration_ms = Some(123);
    comparison.metrics.startup_time.status = "timeout".to_string();
    comparison.metrics.startup_time.duration_ms = Some(30_000);

    let value = compared(run(), comparison);
    let build = &value["metrics"]["build_time"];
    let startup = &value["metrics"]["startup_time"];

    assert_eq!(build["comparison_status"], json!("failed"));
    assert_eq!(build["duration_ms"]["comparison_value"], json!(null));
    assert_eq!(build["duration_ms"]["absolute_difference"], json!(null));
    assert_eq!(startup["comparison_status"], json!("timeout"));
    assert_eq!(startup["duration_ms"]["comparison_value"], json!(null));
}

#[test]
fn skipped_steps_do_not_fail_the_comparison_or_become_zero() {
    let value = compared(run(), skipped(Uuid::now_v7()));
    let metrics = &value["metrics"];

    assert_eq!(
        metrics["build_time"]["comparison_status"],
        json!("no_build_command")
    );
    assert_eq!(
        metrics["startup_time"]["comparison_status"],
        json!("no_start_command")
    );
    assert_eq!(
        metrics["build_time"]["duration_ms"]["comparison_value"],
        json!(null)
    );
    assert_eq!(
        metrics["startup_time"]["duration_ms"]["comparison_value"],
        json!(null)
    );
    assert_eq!(
        metrics["memory"]["peak_rss_bytes"]["comparison_value"],
        json!(null)
    );
    assert_eq!(
        metrics["cpu"]["average_percent"]["comparison_value"],
        json!(null)
    );
    assert_eq!(metrics["http_latency"]["comparison_status"], json!(null));
    assert_eq!(
        metrics["http_latency"]["p95_ms"]["comparison_value"],
        json!(null)
    );
    assert_eq!(value["comparison"]["runtime"], json!(null));

    // The metrics both runs have are still compared
    assert_eq!(
        metrics["artifact_size"]["source"]["bytes"]["absolute_difference"],
        json!(0)
    );
}

#[test]
fn a_partial_http_run_is_not_comparable() {
    let mut comparison = run();

    comparison.metrics.http_latency.as_mut().unwrap().status = "target_exited".to_string();

    let value = compared(run(), comparison);
    let http = &value["metrics"]["http_latency"];

    assert_eq!(http["comparison_status"], json!("target_exited"));
    assert_eq!(http["requests_per_second"]["comparison_value"], json!(null));
    assert_eq!(http["p50_ms"]["comparison_value"], json!(null));
    assert_eq!(http["p95_ms"]["absolute_difference"], json!(null));
    assert_eq!(http["p99_ms"]["percentage_change"], json!(null));
}

#[test]
fn zero_observed_requests_do_not_become_zero_valued_statistics() {
    let mut comparison = run();
    let latency = comparison.metrics.http_latency.as_mut().unwrap();

    latency.total_requests = 0;
    latency.successful_requests = 0;
    latency.failed_requests = 0;
    latency.requests_per_second = 0.0;
    latency.min_ms = None;
    latency.mean_ms = None;
    latency.p50_ms = None;
    latency.p95_ms = None;
    latency.p99_ms = None;
    latency.max_ms = None;

    let value = compared(run(), comparison);
    let http = &value["metrics"]["http_latency"];

    assert_eq!(http["comparison_status"], json!("completed"));

    for field in ["requests_per_second", "p50_ms", "p95_ms", "p99_ms"] {
        assert_eq!(http[field]["comparison_value"], json!(null), "{field}");
        assert_eq!(http[field]["absolute_difference"], json!(null), "{field}");
        assert_eq!(http[field]["percentage_change"], json!(null), "{field}");
    }
}

#[test]
fn directions_are_decided_by_the_backend() {
    let value = compared(run(), run());
    let metrics = &value["metrics"];

    let lower = [
        &metrics["build_time"]["duration_ms"],
        &metrics["startup_time"]["duration_ms"],
        &metrics["memory"]["peak_rss_bytes"],
        &metrics["cpu"]["average_percent"],
        &metrics["http_latency"]["p50_ms"],
        &metrics["http_latency"]["p95_ms"],
        &metrics["http_latency"]["p99_ms"],
        &metrics["artifact_size"]["source"]["bytes"],
        &metrics["artifact_size"]["build"]["file_count"],
        &metrics["artifact_size"]["excluded"]["bytes"],
    ];

    for metric in lower {
        assert_eq!(metric["direction"], json!("lower_is_better"));
    }

    assert_eq!(
        metrics["http_latency"]["requests_per_second"]["direction"],
        json!("higher_is_better")
    );
}
