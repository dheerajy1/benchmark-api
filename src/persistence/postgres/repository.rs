use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    features::v1::benchmark::{configuration::RunConfiguration, model::BenchmarkResult},
    persistence::{
        error::PersistenceError,
        history::SummaryQuery,
        repository::{
            BenchmarkRepository, ConfigurationFuture, ConfigurationListFuture, CountFuture,
            GetFuture, ListFuture, PersistFuture, RunsFuture, TrendsFuture,
        },
    },
};

use super::{detail, insert, mapping, overview, run_configurations, summary};

pub struct PgBenchmarkRepository {
    pool: PgPool,
}

impl PgBenchmarkRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn insert(
        &self,
        result: &BenchmarkResult,
        configuration: &RunConfiguration,
    ) -> Result<(), PersistenceError> {
        // Mapping fails before any connection is used, so nothing is half written
        let rows = mapping::map_result(result)?;

        let mut transaction = self.pool.begin().await?;

        // Any error returns here and the dropped transaction rolls everything back
        insert::insert_all(&mut transaction, &rows).await?;

        // Written in the same transaction, so a run is never stored without its configuration
        run_configurations::insert(&mut transaction, result.run_id, configuration).await?;

        transaction.commit().await?;

        Ok(())
    }
}

impl BenchmarkRepository for PgBenchmarkRepository {
    fn save<'a>(
        &'a self,
        result: &'a BenchmarkResult,
        configuration: &'a RunConfiguration,
    ) -> PersistFuture<'a> {
        Box::pin(self.insert(result, configuration))
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

    fn get_configuration<'a>(&'a self, run_id: Uuid) -> ConfigurationFuture<'a> {
        Box::pin(run_configurations::get(&self.pool, run_id))
    }

    fn list_configurations<'a>(&'a self, limit: i64) -> ConfigurationListFuture<'a> {
        Box::pin(run_configurations::list(&self.pool, limit))
    }
}
