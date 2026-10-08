use uuid::Uuid;

use crate::persistence::{repository::BenchmarkRepository, stored_run::StoredRun};

use super::{
    compare::compare as compare_runs, error::ComparisonError, model::ComparisonResponse,
    query::RunIds,
};

// Loads exactly the two requested runs and compares them, nothing is stored
pub async fn compare(
    repository: &dyn BenchmarkRepository,
    ids: RunIds,
) -> Result<ComparisonResponse, ComparisonError> {
    let runs = repository.get_runs(&[ids.baseline, ids.comparison]).await?;

    let find = |id: Uuid| runs.iter().find(|run: &&StoredRun| run.result.run_id == id);

    match (find(ids.baseline), find(ids.comparison)) {
        (Some(baseline), Some(comparison)) => Ok(compare_runs(baseline, comparison)),
        (baseline, comparison) => {
            // A missing run is never compared as an empty run
            let mut missing = Vec::new();

            if baseline.is_none() {
                missing.push(ids.baseline.to_string());
            }

            if comparison.is_none() {
                missing.push(ids.comparison.to_string());
            }

            let noun = if missing.len() == 1 { "run" } else { "runs" };

            Err(ComparisonError::NotFound(format!(
                "Benchmark {noun} not found: {}.",
                missing.join(", ")
            )))
        }
    }
}
