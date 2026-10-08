use chrono::{DateTime, Utc};

use crate::features::v1::benchmark::model::BenchmarkResult;

// One stored run rebuilt in the POST result shape, plus the stored created_at
#[derive(Clone)]
pub struct StoredRun {
    pub created_at: DateTime<Utc>,
    pub result: BenchmarkResult,
}
