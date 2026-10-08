use std::sync::Mutex;

use benchmark_api::{
    features::v1::benchmark::model::BenchmarkResult,
    persistence::{
        error::PersistenceError,
        repository::{BenchmarkRepository, PersistFuture},
    },
};
use uuid::Uuid;

// Records the run ids it is asked to save, so API tests need no database
#[derive(Default)]
pub struct InMemoryRepository {
    saved: Mutex<Vec<Uuid>>,
}

impl InMemoryRepository {
    pub fn saved_run_ids(&self) -> Vec<Uuid> {
        self.saved.lock().unwrap().clone()
    }
}

impl BenchmarkRepository for InMemoryRepository {
    fn save<'a>(&'a self, result: &'a BenchmarkResult) -> PersistFuture<'a> {
        Box::pin(async move {
            self.saved.lock().unwrap().push(result.run_id);

            Ok(())
        })
    }
}

// Always fails, to prove the API does not report a benchmark that was not stored
pub struct FailingRepository;

impl BenchmarkRepository for FailingRepository {
    fn save<'a>(&'a self, _result: &'a BenchmarkResult) -> PersistFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }
}
