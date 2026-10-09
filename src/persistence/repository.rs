use std::{future::Future, pin::Pin};

use uuid::Uuid;

use crate::features::v1::benchmark::model::BenchmarkResult;

use super::{
    error::PersistenceError,
    history::{SummaryQuery, SummaryRow, TrendRow},
    stored_run::StoredRun,
};

pub type PersistFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), PersistenceError>> + Send + 'a>>;

pub type ListFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SummaryRow>, PersistenceError>> + Send + 'a>>;

pub type GetFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<BenchmarkResult>, PersistenceError>> + Send + 'a>>;

pub type RunsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<StoredRun>, PersistenceError>> + Send + 'a>>;

pub type CountFuture<'a> = Pin<Box<dyn Future<Output = Result<i64, PersistenceError>> + Send + 'a>>;

pub type TrendsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<TrendRow>, PersistenceError>> + Send + 'a>>;

// The boundary between the benchmark domain and storage, the handlers only know this trait
pub trait BenchmarkRepository: Send + Sync {
    // Stores one complete benchmark execution atomically, all of it or nothing
    fn save<'a>(&'a self, result: &'a BenchmarkResult) -> PersistFuture<'a>;

    // Lists history summaries, filtered and paginated by the database, newest first
    fn list_summaries<'a>(&'a self, query: &'a SummaryQuery) -> ListFuture<'a>;

    // Rebuilds one stored benchmark in the POST result shape, None when the run does not exist
    fn get_by_run_id<'a>(&'a self, run_id: Uuid) -> GetFuture<'a>;

    // Rebuilds exactly the requested runs, absent ids are missing from the result and the
    // order of the result is not the order of the ids
    fn get_runs<'a>(&'a self, run_ids: &'a [Uuid]) -> RunsFuture<'a>;

    // Counts every stored run, no filter
    fn count_runs<'a>(&'a self) -> CountFuture<'a>;

    // The newest runs for the overview trends, newest first, with the comparison rules applied
    fn list_trends<'a>(&'a self, limit: i64) -> TrendsFuture<'a>;
}
