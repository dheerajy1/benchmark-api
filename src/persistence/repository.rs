use std::{future::Future, pin::Pin};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::features::v1::benchmark::{
    configuration::{RunConfiguration, StartupSummary},
    model::BenchmarkResult,
};

use super::{
    error::PersistenceError,
    history::{
        HistoryCounts, ProjectRow, ProjectTotals, StageRun, SummaryQuery, SummaryRow, TrendQuery,
        TrendRow,
    },
    outcome::RunOutcome,
    stored_run::StoredRun,
};

pub type PersistFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), PersistenceError>> + Send + 'a>>;

pub type ListFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SummaryRow>, PersistenceError>> + Send + 'a>>;

pub type ProjectsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<ProjectRow>, PersistenceError>> + Send + 'a>>;

pub type ProjectTotalsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ProjectTotals, PersistenceError>> + Send + 'a>>;

pub type StageRunsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<StageRun>, PersistenceError>> + Send + 'a>>;

pub type PingFuture<'a> = Pin<Box<dyn Future<Output = Result<(), PersistenceError>> + Send + 'a>>;

pub type DeleteFuture<'a> =
    Pin<Box<dyn Future<Output = Result<bool, PersistenceError>> + Send + 'a>>;

pub type HistoryCountsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<HistoryCounts, PersistenceError>> + Send + 'a>>;

pub type GetFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<BenchmarkResult>, PersistenceError>> + Send + 'a>>;

pub type RunsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<StoredRun>, PersistenceError>> + Send + 'a>>;

pub type CountFuture<'a> = Pin<Box<dyn Future<Output = Result<i64, PersistenceError>> + Send + 'a>>;

pub type TrendsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<TrendRow>, PersistenceError>> + Send + 'a>>;

pub type ConfigurationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ConfigurationLookup, PersistenceError>> + Send + 'a>>;

pub type ConfigurationListFuture<'a> = Pin<
    Box<dyn Future<Output = Result<Vec<StoredConfigurationRun>, PersistenceError>> + Send + 'a>,
>;

// What a configuration lookup found for one run id
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationLookup {
    // No run with this id
    RunMissing,
    // The run exists but was stored before configurations were recorded
    NotRecorded,
    Recorded(Box<RunConfiguration>),
}

// One run in the configuration listing, newest first
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredConfigurationRun {
    pub run_id: Uuid,
    pub created_at: DateTime<Utc>,
    // done, failed or cancelled
    pub status: String,
    // The run's path: from its configuration when recorded, otherwise from the run row
    pub path: String,
    // The start command and readiness: from the configuration when recorded, otherwise from
    // the run's startup row. None when the run had no start command.
    pub startup: Option<StartupSummary>,
    pub configuration: Option<RunConfiguration>,
}

// The boundary between the benchmark domain and storage, the handlers only know this trait
pub trait BenchmarkRepository: Send + Sync {
    // Stores one benchmark execution atomically, all of it or nothing, with the configuration
    // it executed with and how it ended. The result may be partial: the parts of a step that
    // did not finish are None. The configuration is part of the same transaction.
    fn save_run<'a>(
        &'a self,
        result: &'a BenchmarkResult,
        configuration: &'a RunConfiguration,
        outcome: RunOutcome,
    ) -> PersistFuture<'a>;

    // Stores a run that ran to its end, its status follows from its steps
    fn save<'a>(
        &'a self,
        result: &'a BenchmarkResult,
        configuration: &'a RunConfiguration,
    ) -> PersistFuture<'a> {
        self.save_run(result, configuration, RunOutcome::from_result(result))
    }

    // Soft deletes a run: every read stops seeing it, the part rows stay. Deleting a run that is
    // already deleted is not an error. False only when the id was never stored.
    fn delete_run<'a>(&'a self, run_id: Uuid) -> DeleteFuture<'a>;

    // Lists history summaries, filtered and paginated by the database, newest first
    fn list_summaries<'a>(&'a self, query: &'a SummaryQuery) -> ListFuture<'a>;

    // The numbers of the history filter pills for the same filters, see HistoryCounts
    fn count_history<'a>(&'a self, query: &'a SummaryQuery) -> HistoryCountsFuture<'a>;

    // Rebuilds one stored benchmark in the POST result shape, None when the run does not exist
    fn get_by_run_id<'a>(&'a self, run_id: Uuid) -> GetFuture<'a>;

    // Rebuilds exactly the requested runs, absent ids are missing from the result and the
    // order of the result is not the order of the ids
    fn get_runs<'a>(&'a self, run_ids: &'a [Uuid]) -> RunsFuture<'a>;

    // Counts every stored run, no filter
    fn count_runs<'a>(&'a self) -> CountFuture<'a>;

    // The newest runs for the overview trends, newest first, with the comparison rules applied
    fn list_trends<'a>(&'a self, query: &'a TrendQuery) -> TrendsFuture<'a>;

    // Every project with at least one run that is not deleted, newest activity first
    fn list_projects<'a>(&'a self) -> ProjectsFuture<'a>;

    // How the runs of one project ended, all zeros for a path without runs
    fn project_totals<'a>(&'a self, target_path: &'a str) -> ProjectTotalsFuture<'a>;

    // The stages of the newest `limit` done runs of a target that are not deleted, newest first
    fn recent_stage_runs<'a>(&'a self, target_path: &'a str, limit: i64) -> StageRunsFuture<'a>;

    // Whether the database answers
    fn ping<'a>(&'a self) -> PingFuture<'a>;

    // The configuration of one run, distinguishing a missing run from an unrecorded one
    fn get_configuration<'a>(&'a self, run_id: Uuid) -> ConfigurationFuture<'a>;

    // The newest runs with their configuration, newest first
    fn list_configurations<'a>(&'a self, limit: i64) -> ConfigurationListFuture<'a>;
}
