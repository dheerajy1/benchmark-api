use benchmark_api::{
    features::v1::{
        benchmark::{
            model::{BenchmarkResult, Cpu, Memory, MemoryReading},
            workspace::result::{ImageSize, WorkspaceBuild, WorkspaceResult, WorkspaceStartup},
        },
        comparison::compare::compare,
    },
    persistence::{postgres::mapping::run_created_at, stored_run::StoredRun},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::comparison_sample::result;

fn stored(result: BenchmarkResult) -> StoredRun {
    StoredRun {
        created_at: run_created_at(result.run_id).unwrap(),
        result,
    }
}

fn reading(bytes: u64) -> MemoryReading {
    MemoryReading {
        rss_bytes: bytes,
        rss_size: format!("{bytes} B"),
    }
}

fn workspace(mode: &str, status: &str, image: u64, peak: u64) -> WorkspaceResult {
    let mut result = WorkspaceResult::empty("subscriber", mode, vec!["TOKEN".to_string()]);

    result.status = status.to_string();

    if status == "success" {
        result.build = Some(WorkspaceBuild {
            status: "success".to_string(),
            duration_ms: 1000,
            exit_code: Some(0),
        });
        result.image = Some(ImageSize {
            bytes: image,
            size: format!("{image} B"),
            disk_usage_bytes: None,
            disk_usage: None,
        });
        result.startup = Some(WorkspaceStartup { duration_ms: 500 });
        result.memory = Some(Memory {
            baseline: reading(peak / 2),
            peak: reading(peak),
            delta: reading(peak / 2),
        });
        result.cpu = Some(Cpu {
            baseline_percent: 1.0,
            average_percent: 2.0,
            peak_percent: 3.0,
        });
    }

    result
}

fn run_with(workspaces: Vec<WorkspaceResult>) -> BenchmarkResult {
    let mut run = result(Uuid::now_v7());

    run.metrics.workspaces = workspaces;

    run
}

fn compared(baseline: BenchmarkResult, comparison: BenchmarkResult) -> Value {
    serde_json::to_value(compare(&stored(baseline), &stored(comparison))).unwrap()
}

#[test]
fn runs_without_workspaces_have_no_workspace_section() {
    let value = compared(run_with(Vec::new()), run_with(Vec::new()));

    assert!(value["metrics"].get("workspaces").is_none());
}

#[test]
fn the_typescript_and_rust_subscribers_are_compared_by_name_and_mode() {
    let typescript = run_with(vec![
        workspace("native", "success", 0, 200_000_000),
        workspace("docker", "success", 400_000_000, 220_000_000),
    ]);
    let rust = run_with(vec![
        workspace("native", "success", 0, 20_000_000),
        workspace("docker", "success", 40_000_000, 25_000_000),
    ]);

    let value = compared(typescript, rust);
    let rows = value["metrics"]["workspaces"].as_array().unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["mode"], json!("native"));
    assert_eq!(rows[1]["mode"], json!("docker"));

    let native = &rows[0];

    assert_eq!(
        native["peak_rss_bytes"]["baseline_value"],
        json!(200_000_000)
    );
    assert_eq!(
        native["peak_rss_bytes"]["comparison_value"],
        json!(20_000_000)
    );
    assert_eq!(
        native["peak_rss_bytes"]["absolute_difference"],
        json!(-180_000_000)
    );
    assert_eq!(native["peak_rss_bytes"]["percentage_change"], json!(-90.0));
    assert_eq!(
        native["peak_rss_bytes"]["direction"],
        json!("lower_is_better")
    );

    let docker = &rows[1];

    assert_eq!(
        docker["image_bytes"]["absolute_difference"],
        json!(-360_000_000)
    );
}

#[test]
fn a_failed_mode_is_not_a_comparable_result_and_a_missing_one_is_null() {
    let baseline = run_with(vec![
        workspace("native", "failed", 0, 0),
        workspace("docker", "success", 100, 1000),
    ]);
    let comparison = run_with(vec![workspace("native", "success", 0, 500)]);

    let value = compared(baseline, comparison);
    let rows = value["metrics"]["workspaces"].as_array().unwrap();

    let native = &rows[0];
    assert_eq!(native["baseline_status"], json!("failed"));
    assert_eq!(native["comparison_status"], json!("success"));
    assert_eq!(native["peak_rss_bytes"]["baseline_value"], Value::Null);
    assert_eq!(native["peak_rss_bytes"]["absolute_difference"], Value::Null);

    let docker = &rows[1];
    assert_eq!(docker["baseline_status"], json!("success"));
    assert_eq!(docker["comparison_status"], Value::Null);
    assert_eq!(docker["image_bytes"]["comparison_value"], Value::Null);
}

#[test]
fn results_serialize_without_env_values_and_with_names() {
    let value = serde_json::to_value(workspace("native", "success", 0, 10)).unwrap();

    assert_eq!(value["env_variable_names"], json!(["TOKEN"]));
}
