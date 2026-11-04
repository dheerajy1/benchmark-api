use chrono::{DateTime, Utc};

use crate::features::v1::benchmark::model::BenchmarkResult;

use super::outcome::{RunOutcome, StageRecord, StoredStatus};

// One stored run rebuilt in the POST result shape, plus what is stored next to it
#[derive(Clone)]
pub struct StoredRun {
    pub created_at: DateTime<Utc>,
    pub result: BenchmarkResult,
    pub status: StoredStatus,
    pub stopped_stage: Option<String>,
    pub failure_message: Option<String>,
    // The saved target name from the run configuration, None for a custom run
    pub target_name: Option<String>,
    // In run order, empty for a run with no stage rows
    pub stages: Vec<StageRecord>,
}

impl StoredRun {
    // A run with the status its steps give it, no stages and no saved target name
    pub fn new(created_at: DateTime<Utc>, result: BenchmarkResult) -> Self {
        let outcome = RunOutcome::from_result(&result);

        Self {
            created_at,
            result,
            status: outcome.status,
            stopped_stage: outcome.stopped_stage,
            failure_message: outcome.failure_message,
            target_name: None,
            stages: Vec::new(),
        }
    }
}
