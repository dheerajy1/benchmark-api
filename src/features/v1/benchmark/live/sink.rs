use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};

use super::{control::RunControl, event::kind, registry::RunEntry};
use crate::features::v1::benchmark::error::BenchmarkError;

// The steps of a run, in the order they happen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Discovery,
    ArtifactSize,
    Build,
    Startup,
    Baseline,
    Workload,
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

// For runs nobody watches, the legacy endpoint and the existing tests
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

// Everything the execution needs besides the request: where progress goes and the
// control handle. Cheap to clone, so later phases can hand it to sampler threads.
#[derive(Clone)]
pub struct RunContext {
    sink: Arc<dyn ProgressSink>,
    control: RunControl,
}

impl RunContext {
    pub fn new(sink: Arc<dyn ProgressSink>) -> Self {
        Self {
            sink,
            control: RunControl::default(),
        }
    }

    // A context that shares the control of a run, so a cancel request reaches the execution
    pub fn with_control(sink: Arc<dyn ProgressSink>, control: RunControl) -> Self {
        Self { sink, control }
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

    pub fn stage_started(&self, stage: Stage) {
        self.sink
            .emit(kind::STAGE_STARTED, json!({ "stage": stage.as_str() }));
    }

    // duration_ms is null when the step has no measured duration
    pub fn stage_completed(&self, stage: Stage, status: &str, duration_ms: Option<u64>) {
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

    pub fn baseline_measured(&self, memory_rss_bytes: u64, cpu_percent: f64) {
        self.sink.emit(
            kind::BASELINE_MEASURED,
            json!({ "memory_rss_bytes": memory_rss_bytes, "cpu_percent": cpu_percent }),
        );
    }
}
