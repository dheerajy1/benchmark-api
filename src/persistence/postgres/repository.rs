use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    features::v1::benchmark::model::BenchmarkResult,
    persistence::{
        error::PersistenceError,
        history::SummaryQuery,
        repository::{
            BenchmarkRepository, CountFuture, GetFuture, ListFuture, PersistFuture, RunsFuture,
            TrendsFuture,
        },
    },
};

use super::{detail, insert, mapping, overview, summary};

pub struct PgBenchmarkRepository {
    pool: PgPool,
}

impl PgBenchmarkRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn insert(&self, result: &BenchmarkResult) -> Result<(), PersistenceError> {
        // Mapping fails before any connection is used, so nothing is half written
        let rows = mapping::map_result(result)?;

        let mut transaction = self.pool.begin().await?;

        // Any error returns here and the dropped transaction rolls everything back
        insert::insert_all(&mut transaction, &rows).await?;

        transaction.commit().await?;

        Ok(())
    }
}

impl BenchmarkRepository for PgBenchmarkRepository {
    fn save<'a>(&'a self, result: &'a BenchmarkResult) -> PersistFuture<'a> {
        Box::pin(self.insert(result))
    }

    fn list_summaries<'a>(&'a self, query: &'a SummaryQuery) -> ListFuture<'a> {
        Box::pin(summary::list(&self.pool, query))
    }

    fn get_by_run_id<'a>(&'a self, run_id: Uuid) -> GetFuture<'a> {
        Box::pin(detail::get(&self.pool, run_id))
    }

    fn get_runs<'a>(&'a self, run_ids: &'a [Uuid]) -> RunsFuture<'a> {
        Box::pin(detail::get_many(&self.pool, run_ids))
    }

    fn count_runs<'a>(&'a self) -> CountFuture<'a> {
        Box::pin(overview::count_runs(&self.pool))
    }

    fn list_trends<'a>(&'a self, limit: i64) -> TrendsFuture<'a> {
        Box::pin(overview::list_trends(&self.pool, limit))
    }
}
