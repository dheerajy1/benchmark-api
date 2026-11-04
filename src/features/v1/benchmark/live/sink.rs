use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use serde::Serialize;
use serde_json::{Value, json};

use super::{control::RunControl, event::kind, registry::RunEntry};
use crate::{features::v1::benchmark::error::BenchmarkError, persistence::outcome::StageRecord};

// The steps of a run, in the order they happen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Discovery,
    ArtifactSize,
    Build,
    Startup,
    Baseline,
    Workload,
    Workspaces,
    Saving,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Discovery => "discovery",
            Stage::ArtifactSize => "artifact_size",
            Stage::Build => "build",
            Stage::Startup => "startup",
            Stage::Baseline => "baseline",
            Stage::Workload => "workload",
            Stage::Workspaces => "workspaces",
            Stage::Saving => "saving",
        }
    }
}

// The measurements that are available as one value when their step ends
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Discovery,
    ArtifactSize,
    BuildTime,
    StartupTime,
}

impl Metric {
    pub fn as_str(self) -> &'static str {
        match self {
            Metric::Discovery => "discovery",
            Metric::ArtifactSize => "artifact_size",
            Metric::BuildTime => "build_time",
            Metric::StartupTime => "startup_time",
        }
    }
}

// Why a stage did not run
pub mod skip {
    pub const NO_WORKLOAD: &str = "no_workload";
    pub const NO_START_COMMAND: &str = "no_start_command";
    pub const STARTUP_NOT_SUCCESSFUL: &str = "startup_not_successful";
    pub const TARGET_EXITED: &str = "target_exited";
}

// Receives the progress of a run. It is called from the blocking execution thread,
// so an implementation must return quickly and never block.
pub trait ProgressSink: Send + Sync {
    fn emit(&self, kind: &'static str, data: Value);

    // Whether the run should measure and emit live samples. A sink nobody watches
    // returns false, so an unobserved run does no sampling work at all.
    fn wants_samples(&self) -> bool {
        false
    }
}

// For runs nobody watches and the existing tests
pub struct NoopSink;

impl ProgressSink for NoopSink {
    fn emit(&self, _kind: &'static str, _data: Value) {}
}

// Appends the progress of one run to its event log
pub struct EntrySink {
    entry: Arc<RunEntry>,
}

impl EntrySink {
    pub fn new(entry: Arc<RunEntry>) -> Self {
        Self { entry }
    }
}

impl ProgressSink for EntrySink {
    fn emit(&self, kind: &'static str, data: Value) {
        // A refused event means the run is already terminal or over its bound, and
        // progress must never be able to fail the benchmark itself
        let _ = self.entry.append(kind, data);
    }

    fn wants_samples(&self) -> bool {
        true
    }
}

struct StageEntry {
    stage: String,
    status: Option<String>,
    duration_ms: Option<u64>,
    started: Instant,
}

// Remembers how long every stage took, in the order the stages started. It is what the run
// stores in benchmark_run_stages.
#[derive(Default)]
pub struct StageRecorder {
    entries: Mutex<Vec<StageEntry>>,
}

impl StageRecorder {
    fn entries(&self) -> MutexGuard<'_, Vec<StageEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn started(&self, key: &str) {
        self.entries().push(StageEntry {
            stage: key.to_string(),
            status: None,
            duration_ms: None,
            started: Instant::now(),
        });
    }

    // The duration the stage reports, or the time since it started when it reports none
    pub fn completed(&self, key: &str, status: &str, duration_ms: Option<u64>) {
        let mut entries = self.entries();

        if let Some(entry) = entries
            .iter_mut()
            .rev()
            .find(|entry| entry.stage == key && entry.status.is_none())
        {
            let elapsed = u64::try_from(entry.started.elapsed().as_millis()).ok();

            entry.status = Some(status.to_string());
            entry.duration_ms = duration_ms.or(elapsed);
        }
    }

    // The finished stages. With an interrupted status, the stages that were still running
    // are added with that status and the time they had run.
    pub fn snapshot(&self, interrupted_status: Option<&str>) -> Vec<StageRecord> {
        self.entries()
            .iter()
            .filter_map(|entry| match (&entry.status, interrupted_status) {
                (Some(status), _) => Some(StageRecord {
                    stage: entry.stage.clone(),
                    status: status.clone(),
                    duration_ms: entry.duration_ms,
                }),
                (None, Some(status)) => Some(StageRecord {
                    stage: entry.stage.clone(),
                    status: status.to_string(),
                    duration_ms: u64::try_from(entry.started.elapsed().as_millis()).ok(),
                }),
                (None, None) => None,
            })
            .collect()
    }
}

// The key of one mode of one workspace in the stage records
pub fn workspace_stage_key(workspace: &str, mode: &str) -> String {
    format!("workspaces:{workspace}:{mode}")
}

// Everything the execution needs besides the request: where progress goes and the
// control handle. Cheap to clone, so later phases can hand it to sampler threads.
#[derive(Clone)]
pub struct RunContext {
    sink: Arc<dyn ProgressSink>,
    control: RunControl,
    stages: Arc<StageRecorder>,
}

impl RunContext {
    pub fn new(sink: Arc<dyn ProgressSink>) -> Self {
        Self {
            sink,
            control: RunControl::default(),
            stages: Arc::new(StageRecorder::default()),
        }
    }

    // A context that shares the control of a run, so a cancel request reaches the execution
    pub fn with_control(sink: Arc<dyn ProgressSink>, control: RunControl) -> Self {
        Self {
            sink,
            control,
            stages: Arc::new(StageRecorder::default()),
        }
    }

    pub fn noop() -> Self {
        Self::new(Arc::new(NoopSink))
    }

    pub fn control(&self) -> &RunControl {
        &self.control
    }

    // Ends the run with Cancelled when a cancel request is pending, for the points between steps
    pub fn check_cancelled(&self) -> Result<(), BenchmarkError> {
        if self.control.is_cancel_requested() {
            Err(BenchmarkError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn samples_enabled(&self) -> bool {
        self.sink.wants_samples()
    }

    // The stage durations of this run so far, see StageRecorder::snapshot
    pub fn stage_records(&self, interrupted_status: Option<&str>) -> Vec<StageRecord> {
        self.stages.snapshot(interrupted_status)
    }

    pub fn stage_started(&self, stage: Stage) {
        self.stages.started(stage.as_str());
        self.sink
            .emit(kind::STAGE_STARTED, json!({ "stage": stage.as_str() }));
    }

    // duration_ms is null when the step has no measured duration
    pub fn stage_completed(&self, stage: Stage, status: &str, duration_ms: Option<u64>) {
        self.stages.completed(stage.as_str(), status, duration_ms);
        self.sink.emit(
            kind::STAGE_COMPLETED,
            json!({ "stage": stage.as_str(), "status": status, "duration_ms": duration_ms }),
        );
    }

    pub fn stage_skipped(&self, stage: Stage, reason: &str) {
        self.sink.emit(
            kind::STAGE_SKIPPED,
            json!({ "stage": stage.as_str(), "reason": reason }),
        );
    }

    // The value is the exact existing model of that metric
    pub fn measurement<T: Serialize>(&self, metric: Metric, value: &T) {
        let value = serde_json::to_value(value).unwrap_or(Value::Null);

        self.sink.emit(
            kind::MEASUREMENT_AVAILABLE,
            json!({ "metric": metric.as_str(), "value": value }),
        );
    }

    // The data comes from live::gauges::build_sample
    pub fn sample(&self, data: Value) {
        self.sink.emit(kind::SAMPLE, data);
    }

    // `phase` is started, sampling or finished, `status` only comes with finished
    pub fn workspace_phase(&self, workspace: &str, mode: &str, phase: &str, status: Option<&str>) {
        let key = workspace_stage_key(workspace, mode);

        match phase {
            "started" => self.stages.started(&key),
            "finished" => self
                .stages
                .completed(&key, status.unwrap_or("success"), None),
            _ => {}
        }

        self.sink.emit(
            kind::WORKSPACE_PHASE,
            json!({ "workspace": workspace, "mode": mode, "phase": phase, "status": status }),
        );
    }

    // The data comes from live::gauges::build_workspace_sample
    pub fn workspace_sample(&self, data: Value) {
        self.sink.emit(kind::WORKSPACE_SAMPLE, data);
    }

    pub fn baseline_measured(&self, memory_rss_bytes: u64, cpu_percent: f64) {
        self.sink.emit(
            kind::BASELINE_MEASURED,
            json!({ "memory_rss_bytes": memory_rss_bytes, "cpu_percent": cpu_percent }),
        );
    }
}
