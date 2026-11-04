use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    features::v1::benchmark::{configuration::RunConfiguration, model::BenchmarkResult},
    persistence::{
        error::PersistenceError,
        history::{SummaryQuery, TrendQuery},
        outcome::RunOutcome,
        repository::{
            BenchmarkRepository, ConfigurationFuture, ConfigurationListFuture, CountFuture,
            DeleteFuture, GetFuture, HistoryCountsFuture, ListFuture, PersistFuture, PingFuture,
            ProjectTotalsFuture, ProjectsFuture, RunsFuture, StageRunsFuture, TrendsFuture,
        },
    },
};

use super::{detail, insert, mapping, overview, removal, run_configurations, stage_runs, summary};

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
        outcome: &RunOutcome,
    ) -> Result<(), PersistenceError> {
        let started = std::time::Instant::now();

        // Mapping fails before any connection is used, so nothing is half written
        let rows = mapping::map_result_with(result, outcome)?;

        let mut transaction = self.pool.begin().await?;

        // Any error returns here and the dropped transaction rolls everything back
        insert::insert_all(&mut transaction, &rows).await?;

        // Written in the same transaction, so a run is never stored without its configuration
        run_configurations::insert(&mut transaction, result.run_id, configuration).await?;

        insert::insert_saving_stage(&mut transaction, &rows, started.elapsed()).await?;

        transaction.commit().await?;

        Ok(())
    }
}

impl BenchmarkRepository for PgBenchmarkRepository {
    fn save_run<'a>(
        &'a self,
        result: &'a BenchmarkResult,
        configuration: &'a RunConfiguration,
        outcome: RunOutcome,
    ) -> PersistFuture<'a> {
        Box::pin(async move { self.insert(result, configuration, &outcome).await })
    }

    fn list_summaries<'a>(&'a self, query: &'a SummaryQuery) -> ListFuture<'a> {
        Box::pin(summary::list(&self.pool, query))
    }

    fn delete_run<'a>(&'a self, run_id: Uuid) -> DeleteFuture<'a> {
        Box::pin(removal::delete(&self.pool, run_id))
    }

    fn count_history<'a>(&'a self, query: &'a SummaryQuery) -> HistoryCountsFuture<'a> {
        Box::pin(summary::count(&self.pool, query))
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

    fn list_trends<'a>(&'a self, query: &'a TrendQuery) -> TrendsFuture<'a> {
        Box::pin(overview::list_trends(&self.pool, query))
    }

    fn list_projects<'a>(&'a self) -> ProjectsFuture<'a> {
        Box::pin(overview::list_projects(&self.pool))
    }

    fn project_totals<'a>(&'a self, target_path: &'a str) -> ProjectTotalsFuture<'a> {
        Box::pin(overview::project_totals(&self.pool, target_path))
    }

    fn recent_stage_runs<'a>(&'a self, target_path: &'a str, limit: i64) -> StageRunsFuture<'a> {
        Box::pin(stage_runs::recent(&self.pool, target_path, limit))
    }

    fn ping<'a>(&'a self) -> PingFuture<'a> {
        Box::pin(stage_runs::ping(&self.pool))
    }

    fn get_configuration<'a>(&'a self, run_id: Uuid) -> ConfigurationFuture<'a> {
        Box::pin(run_configurations::get(&self.pool, run_id))
    }

    fn list_configurations<'a>(&'a self, limit: i64) -> ConfigurationListFuture<'a> {
        Box::pin(run_configurations::list(&self.pool, limit))
    }
}
