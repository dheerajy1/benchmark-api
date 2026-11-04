use crate::features::v1::benchmark::model::BenchmarkResult;

pub const STATUS_DONE: &str = "done";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_CANCELLED: &str = "cancelled";

// Step statuses that make a run failed
const FAILING_STATUSES: [&str; 4] = ["failed", "timeout", "unhealthy", "target_exited"];

// The run level status, set once when the run is saved
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredStatus {
    Done,
    Failed,
    Cancelled,
}

impl StoredStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StoredStatus::Done => STATUS_DONE,
            StoredStatus::Failed => STATUS_FAILED,
            StoredStatus::Cancelled => STATUS_CANCELLED,
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            STATUS_DONE => Some(StoredStatus::Done),
            STATUS_FAILED => Some(StoredStatus::Failed),
            STATUS_CANCELLED => Some(StoredStatus::Cancelled),
            _ => None,
        }
    }
}

// How long one stage took. The stage is one of the 8 stage keys, or
// workspaces:<name>:native and workspaces:<name>:docker for one mode of one workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRecord {
    pub stage: String,
    pub status: String,
    pub duration_ms: Option<u64>,
}

// How a run ended, stored next to its measurements
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub status: StoredStatus,
    pub stopped_stage: Option<String>,
    pub failure_message: Option<String>,
    pub stages: Vec<StageRecord>,
}

// The first step that failed, in run order
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFailure {
    pub stage: &'static str,
    pub message: String,
}

impl RunOutcome {
    // The status of a run that ran to its end: failed when a step that ran failed, otherwise
    // done. Steps that did not apply (no_build_command, no_start_command, no_port) and a run
    // with no workload never fail a run, and neither does a failed workspace mode.
    pub fn from_result(result: &BenchmarkResult) -> Self {
        match step_failure(result) {
            None => Self {
                status: StoredStatus::Done,
                stopped_stage: None,
                failure_message: None,
                stages: Vec::new(),
            },
            Some(failure) => Self {
                status: StoredStatus::Failed,
                stopped_stage: Some(failure.stage.to_string()),
                failure_message: Some(failure.message),
                stages: Vec::new(),
            },
        }
    }

    // The run failed on its own
    pub fn failed(stage: Option<String>, message: String, stages: Vec<StageRecord>) -> Self {
        Self {
            status: StoredStatus::Failed,
            stopped_stage: stage,
            failure_message: Some(message),
            stages,
        }
    }

    // The run was stopped on request
    pub fn cancelled(stage: Option<String>, stages: Vec<StageRecord>) -> Self {
        Self {
            status: StoredStatus::Cancelled,
            stopped_stage: stage,
            failure_message: None,
            stages,
        }
    }

    pub fn with_stages(mut self, stages: Vec<StageRecord>) -> Self {
        self.stages = stages;

        self
    }
}

// Looks at build, startup and the HTTP load of the main project
pub fn step_failure(result: &BenchmarkResult) -> Option<StepFailure> {
    let metrics = &result.metrics;

    let steps = [
        (
            "build",
            "build",
            metrics.build_time.as_ref().map(|step| step.status.as_str()),
        ),
        (
            "startup",
            "startup",
            metrics
                .startup_time
                .as_ref()
                .map(|step| step.status.as_str()),
        ),
        (
            "workload",
            "load",
            metrics
                .http_latency
                .as_ref()
                .map(|step| step.status.as_str()),
        ),
    ];

    for (stage, label, status) in steps {
        let Some(status) = status else {
            continue;
        };

        if FAILING_STATUSES.contains(&status) {
            return Some(StepFailure {
                stage,
                message: format!("The {label} {}.", phrase(status)),
            });
        }
    }

    None
}

fn phrase(status: &str) -> &'static str {
    match status {
        "timeout" => "timed out",
        "unhealthy" => "was unhealthy",
        "target_exited" => "stopped because the target exited",
        _ => "failed",
    }
}
