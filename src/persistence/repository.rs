use std::{future::Future, pin::Pin};

use crate::features::v1::benchmark::model::BenchmarkResult;

use super::error::PersistenceError;

pub type PersistFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), PersistenceError>> + Send + 'a>>;

// The boundary between the benchmark domain and storage, the handler only knows this trait
pub trait BenchmarkRepository: Send + Sync {
    // Stores one complete benchmark execution atomically, all of it or nothing
    fn save<'a>(&'a self, result: &'a BenchmarkResult) -> PersistFuture<'a>;
}
