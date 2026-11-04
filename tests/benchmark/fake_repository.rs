use std::sync::Mutex;

use benchmark_api::{
    features::v1::benchmark::{metrics::http_latency::STATUS_COMPLETED, model::BenchmarkResult},
    persistence::{
        error::PersistenceError,
        history::{
            HistoryCounts, ProjectRow, ProjectTotals, StageDuration, StageRun, SummaryQuery,
            SummaryRow, TrendQuery, TrendRow, WorkspaceSummaryRow,
        },
        outcome::{RunOutcome, StoredStatus},
        postgres::mapping::{STATUS_SUCCESS, run_created_at},
        repository::{
            BenchmarkRepository, CountFuture, DeleteFuture, GetFuture, HistoryCountsFuture,
            ListFuture, PersistFuture, PingFuture, ProjectTotalsFuture, ProjectsFuture, RunsFuture,
            StageRunsFuture, TrendsFuture,
        },
        stored_run::StoredRun,
    },
};
use uuid::Uuid;

// The runtime codes the seed migration inserts, the fake rejects any other code like SQL does
const KNOWN_RUNTIMES: [&str; 5] = ["bun", "node", "python", "rust", "go"];

// The stage rows of one run: its id and each stage with its duration
type StageEntry = (Uuid, Vec<(String, Option<u64>)>);

// Keeps what it is asked to save, so API tests need no database and can read it back
#[derive(Default)]
pub struct InMemoryRepository {
    saved: Mutex<Vec<Uuid>>,
    results: Mutex<Vec<BenchmarkResult>>,
    extra_summaries: Mutex<Vec<SummaryRow>>,
    deleted: Mutex<Vec<Uuid>>,
    stages: Mutex<Vec<StageEntry>>,
}

impl InMemoryRepository {
    pub fn saved_run_ids(&self) -> Vec<Uuid> {
        self.saved.lock().unwrap().clone()
    }

    // Adds a complete result without going through save, to control every metric exactly
    pub fn seed_result(&self, result: BenchmarkResult) {
        self.results.lock().unwrap().push(result);
    }

    // Gives a run its stage rows in run order, to control the stage averages exactly
    pub fn seed_stages(&self, run_id: Uuid, stages: &[(&str, Option<u64>)]) {
        self.stages.lock().unwrap().push((
            run_id,
            stages
                .iter()
                .map(|(stage, duration)| ((*stage).to_string(), *duration))
                .collect(),
        ));
    }

    // Whether a run was soft deleted, a deleted run is hidden from every read
    fn is_deleted(&self, run_id: Uuid) -> bool {
        self.deleted.lock().unwrap().contains(&run_id)
    }

    // Adds a history row without a full result, to control ids and timestamps exactly
    pub fn seed_summary(&self, row: SummaryRow) {
        self.extra_summaries.lock().unwrap().push(row);
    }
}

// Mirrors the SQL of the real repository: filters, cursor, ordering and limit
fn list(
    repository: &InMemoryRepository,
    query: &SummaryQuery,
) -> Result<Vec<SummaryRow>, PersistenceError> {
    let mut rows = matching(repository, query, false, false)?;

    rows.retain(|row| {
        query
            .after
            .as_ref()
            .is_none_or(|after| (row.created_at, row.id) < (after.created_at, after.id))
    });

    rows.sort_by_key(|row| std::cmp::Reverse((row.created_at, row.id)));
    rows.truncate(query.limit as usize);

    Ok(rows)
}

// Every run the filters let through, a facet's own filter can be left out like the counts do
fn matching(
    repository: &InMemoryRepository,
    query: &SummaryQuery,
    skip_runtime: bool,
    skip_status: bool,
) -> Result<Vec<SummaryRow>, PersistenceError> {
    if let Some(code) = &query.runtime_code
        && !KNOWN_RUNTIMES.contains(&code.as_str())
    {
        return Err(PersistenceError::UnknownLookup {
            table: "runtimes",
            code: code.clone(),
        });
    }

    let mut rows: Vec<SummaryRow> = repository
        .results
        .lock()
        .unwrap()
        .iter()
        .map(summary_of)
        .collect();

    rows.extend(repository.extra_summaries.lock().unwrap().iter().cloned());

    rows.retain(|row| {
        let visible = !repository.is_deleted(row.id);
        let runtime_matches = skip_runtime
            || query
                .runtime_code
                .as_ref()
                .is_none_or(|code| row.runtime_code.as_ref() == Some(code));

        let status_matches = skip_status || query.status.is_none_or(|status| row.status == status);

        let path_matches = query
            .target_path
            .as_ref()
            .is_none_or(|path| &row.target_path == path);

        let search_matches = query.search.as_ref().is_none_or(|text| {
            let text = text.to_lowercase();

            row.target_path.to_lowercase().contains(&text)
                || row.id.to_string().to_lowercase().contains(&text)
        });

        visible && runtime_matches && status_matches && path_matches && search_matches
    });

    Ok(rows)
}

fn counts(
    repository: &InMemoryRepository,
    query: &SummaryQuery,
) -> Result<HistoryCounts, PersistenceError> {
    let total = matching(repository, query, false, false)?.len() as i64;

    let mut by_runtime: Vec<(String, i64)> = Vec::new();

    for row in matching(repository, query, true, false)? {
        let code = row.runtime_code.unwrap_or_else(|| "unknown".to_string());

        match by_runtime.iter_mut().find(|(known, _)| *known == code) {
            Some((_, runs)) => *runs += 1,
            None => by_runtime.push((code, 1)),
        }
    }

    by_runtime.sort();

    let by_status_rows = matching(repository, query, false, true)?;

    let by_status = [
        StoredStatus::Done,
        StoredStatus::Failed,
        StoredStatus::Cancelled,
    ]
    .into_iter()
    .map(|status| {
        (
            status,
            by_status_rows
                .iter()
                .filter(|row| row.status == status)
                .count() as i64,
        )
    })
    .collect();

    Ok(HistoryCounts {
        total,
        by_runtime,
        by_status,
    })
}

fn summary_of(result: &BenchmarkResult) -> SummaryRow {
    let metrics = &result.metrics;

    SummaryRow {
        id: result.run_id,
        target_path: result.target.path.clone(),
        target_name: None,
        runtime_code: result.discovery.runtime.clone(),
        status: RunOutcome::from_result(result).status,
        created_at: run_created_at(result.run_id).unwrap(),
        build_output_bytes: metrics
            .artifact_size
            .as_ref()
            .map(|size| size.build.bytes as i64),
        workspaces: metrics
            .workspaces
            .iter()
            .map(|workspace| WorkspaceSummaryRow {
                name: workspace.name.clone(),
                mode: workspace.mode.clone(),
                status: workspace.status.clone(),
                image_bytes: workspace.image.as_ref().map(|image| image.bytes as i64),
                disk_usage_bytes: workspace
                    .image
                    .as_ref()
                    .and_then(|image| image.disk_usage_bytes)
                    .map(|bytes| bytes as i64),
                build_duration_ms: workspace
                    .build
                    .as_ref()
                    .map(|build| build.duration_ms as i64),
            })
            .collect(),
        build_time_ms: metrics
            .build_time
            .as_ref()
            .and_then(|build| build.duration_ms)
            .map(|value| value as i64),
        startup_time_ms: metrics
            .startup_time
            .as_ref()
            .and_then(|startup| startup.duration_ms)
            .map(|value| value as i64),
        memory_peak_bytes: metrics
            .memory
            .as_ref()
            .map(|memory| memory.peak.rss_bytes as i64),
        cpu_average_percent: metrics.cpu.as_ref().map(|cpu| cpu.average_percent),
        requests_per_second: metrics
            .http_latency
            .as_ref()
            .map(|latency| latency.requests_per_second),
        p95_ms: metrics
            .http_latency
            .as_ref()
            .and_then(|latency| latency.p95_ms),
    }
}

// Mirrors the SQL of the overview trends: newest first and the comparison rules applied.
// Only complete results are used, a seeded summary row has no statuses to apply the rules to.
fn trends(repository: &InMemoryRepository, query: &TrendQuery) -> Vec<TrendRow> {
    let mut rows: Vec<TrendRow> = repository
        .results
        .lock()
        .unwrap()
        .iter()
        .filter(|result| !repository.is_deleted(result.run_id))
        .filter(|result| {
            query
                .target_path
                .as_ref()
                .is_none_or(|path| &result.target.path == path)
        })
        .filter(|result| {
            query
                .status
                .is_none_or(|status| RunOutcome::from_result(result).status == status)
        })
        .map(trend_of)
        .collect();

    rows.sort_by_key(|row| std::cmp::Reverse((row.created_at, row.id)));
    rows.truncate(usize::try_from(query.limit).unwrap_or(0));

    rows
}

// Every visible run as a history row, seeded rows included
fn visible_rows(repository: &InMemoryRepository) -> Vec<SummaryRow> {
    let mut rows: Vec<SummaryRow> = repository
        .results
        .lock()
        .unwrap()
        .iter()
        .map(summary_of)
        .collect();

    rows.extend(repository.extra_summaries.lock().unwrap().iter().cloned());
    rows.retain(|row| !repository.is_deleted(row.id));
    rows.sort_by_key(|row| std::cmp::Reverse((row.created_at, row.id)));

    rows
}

// Mirrors the SQL of the project list: one entry per path, newest activity first
fn projects(repository: &InMemoryRepository) -> Vec<ProjectRow> {
    let rows = visible_rows(repository);
    let mut projects: Vec<ProjectRow> = Vec::new();

    // Rows are newest first, so the first row of a path is its newest run
    for row in &rows {
        if projects
            .iter()
            .any(|project| project.path == row.target_path)
        {
            continue;
        }

        let of_path: Vec<&SummaryRow> = rows
            .iter()
            .filter(|other| other.target_path == row.target_path)
            .collect();

        projects.push(ProjectRow {
            path: row.target_path.clone(),
            target_name: row.target_name.clone(),
            runtime_code: of_path.iter().find_map(|other| other.runtime_code.clone()),
            source: None,
            total_runs: of_path.len() as i64,
            done_runs: of_path
                .iter()
                .filter(|other| other.status == StoredStatus::Done)
                .count() as i64,
            last_run_at: row.created_at,
        });
    }

    projects
}

fn project_totals(repository: &InMemoryRepository, target_path: &str) -> ProjectTotals {
    let rows = visible_rows(repository);
    let count = |status: StoredStatus| {
        rows.iter()
            .filter(|row| row.target_path == target_path && row.status == status)
            .count() as i64
    };

    ProjectTotals {
        total_runs: rows
            .iter()
            .filter(|row| row.target_path == target_path)
            .count() as i64,
        done_runs: count(StoredStatus::Done),
        failed_runs: count(StoredStatus::Failed),
        cancelled_runs: count(StoredStatus::Cancelled),
    }
}

fn trend_of(result: &BenchmarkResult) -> TrendRow {
    let metrics = &result.metrics;

    // Only a completed workload with at least one request is a comparable HTTP result
    let comparable_http = metrics
        .http_latency
        .as_ref()
        .filter(|latency| latency.status == STATUS_COMPLETED && latency.total_requests > 0);

    TrendRow {
        id: result.run_id,
        created_at: run_created_at(result.run_id).unwrap(),
        build_time_ms: metrics
            .build_time
            .as_ref()
            .and_then(|build| successful_duration(&build.status, build.duration_ms)),
        startup_time_ms: metrics
            .startup_time
            .as_ref()
            .and_then(|startup| successful_duration(&startup.status, startup.duration_ms)),
        memory_peak_bytes: metrics
            .memory
            .as_ref()
            .map(|memory| memory.peak.rss_bytes as i64),
        cpu_average_percent: metrics.cpu.as_ref().map(|cpu| cpu.average_percent),
        requests_per_second: comparable_http.map(|latency| latency.requests_per_second),
        p95_ms: comparable_http.and_then(|latency| latency.p95_ms),
    }
}

// A duration only counts when the step succeeded
fn successful_duration(status: &str, duration_ms: Option<u64>) -> Option<i64> {
    if status == STATUS_SUCCESS {
        duration_ms.map(|value| value as i64)
    } else {
        None
    }
}

impl BenchmarkRepository for InMemoryRepository {
    fn save_run<'a>(
        &'a self,
        result: &'a BenchmarkResult,
        _configuration: &'a benchmark_api::features::v1::benchmark::configuration::RunConfiguration,
        outcome: benchmark_api::persistence::outcome::RunOutcome,
    ) -> PersistFuture<'a> {
        Box::pin(async move {
            self.stages.lock().unwrap().push((
                result.run_id,
                outcome
                    .stages
                    .iter()
                    .map(|stage| (stage.stage.clone(), stage.duration_ms))
                    .collect(),
            ));
            self.saved.lock().unwrap().push(result.run_id);
            self.results.lock().unwrap().push(result.clone());

            Ok(())
        })
    }

    fn get_configuration<'a>(
        &'a self,
        _run_id: uuid::Uuid,
    ) -> benchmark_api::persistence::repository::ConfigurationFuture<'a> {
        Box::pin(async {
            Ok(benchmark_api::persistence::repository::ConfigurationLookup::RunMissing)
        })
    }

    fn list_configurations<'a>(
        &'a self,
        _limit: i64,
    ) -> benchmark_api::persistence::repository::ConfigurationListFuture<'a> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn list_summaries<'a>(&'a self, query: &'a SummaryQuery) -> ListFuture<'a> {
        Box::pin(async move { list(self, query) })
    }

    fn delete_run<'a>(&'a self, run_id: Uuid) -> DeleteFuture<'a> {
        Box::pin(async move {
            let known = self
                .results
                .lock()
                .unwrap()
                .iter()
                .any(|result| result.run_id == run_id)
                || self
                    .extra_summaries
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|row| row.id == run_id);

            if known && !self.is_deleted(run_id) {
                self.deleted.lock().unwrap().push(run_id);
            }

            Ok(known)
        })
    }

    fn count_history<'a>(&'a self, query: &'a SummaryQuery) -> HistoryCountsFuture<'a> {
        Box::pin(async move { counts(self, query) })
    }

    fn get_runs<'a>(&'a self, run_ids: &'a [Uuid]) -> RunsFuture<'a> {
        Box::pin(async move { Ok(self.runs(run_ids)) })
    }

    fn get_by_run_id<'a>(&'a self, run_id: Uuid) -> GetFuture<'a> {
        Box::pin(async move {
            Ok(self
                .results
                .lock()
                .unwrap()
                .iter()
                .find(|result| result.run_id == run_id && !self.is_deleted(run_id))
                .cloned())
        })
    }

    fn count_runs<'a>(&'a self) -> CountFuture<'a> {
        Box::pin(async move {
            let results = self
                .results
                .lock()
                .unwrap()
                .iter()
                .filter(|result| !self.is_deleted(result.run_id))
                .count();

            let summaries = self
                .extra_summaries
                .lock()
                .unwrap()
                .iter()
                .filter(|row| !self.is_deleted(row.id))
                .count();

            Ok((results + summaries) as i64)
        })
    }

    fn list_trends<'a>(&'a self, query: &'a TrendQuery) -> TrendsFuture<'a> {
        Box::pin(async move { Ok(trends(self, query)) })
    }

    fn list_projects<'a>(&'a self) -> ProjectsFuture<'a> {
        Box::pin(async move { Ok(projects(self)) })
    }

    fn project_totals<'a>(&'a self, target_path: &'a str) -> ProjectTotalsFuture<'a> {
        Box::pin(async move { Ok(project_totals(self, target_path)) })
    }

    fn recent_stage_runs<'a>(&'a self, target_path: &'a str, limit: i64) -> StageRunsFuture<'a> {
        Box::pin(async move {
            let mut rows = visible_rows(self);

            rows.retain(|row| row.target_path == target_path && row.status == StoredStatus::Done);
            rows.truncate(usize::try_from(limit).unwrap_or(0));

            let stages = self.stages.lock().unwrap();

            Ok(rows
                .iter()
                .map(|row| StageRun {
                    run_id: row.id,
                    stages: stages
                        .iter()
                        .find(|(run_id, _)| *run_id == row.id)
                        .map(|(_, list)| {
                            list.iter()
                                .enumerate()
                                .map(|(position, (stage, duration))| StageDuration {
                                    stage: stage.clone(),
                                    position: position as i32,
                                    duration_ms: duration.map(|value| value as i64),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect())
        })
    }

    fn ping<'a>(&'a self) -> PingFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

impl InMemoryRepository {
    fn runs(&self, run_ids: &[Uuid]) -> Vec<StoredRun> {
        self.results
            .lock()
            .unwrap()
            .iter()
            .filter(|result| run_ids.contains(&result.run_id) && !self.is_deleted(result.run_id))
            .map(|result| StoredRun::new(run_created_at(result.run_id).unwrap(), result.clone()))
            .collect()
    }
}

// Always fails, to prove the API does not report a benchmark that was not stored
pub struct FailingRepository;

impl BenchmarkRepository for FailingRepository {
    fn save_run<'a>(
        &'a self,
        _result: &'a BenchmarkResult,
        _configuration: &'a benchmark_api::features::v1::benchmark::configuration::RunConfiguration,
        _outcome: benchmark_api::persistence::outcome::RunOutcome,
    ) -> PersistFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn get_configuration<'a>(
        &'a self,
        _run_id: uuid::Uuid,
    ) -> benchmark_api::persistence::repository::ConfigurationFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn list_configurations<'a>(
        &'a self,
        _limit: i64,
    ) -> benchmark_api::persistence::repository::ConfigurationListFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn list_summaries<'a>(&'a self, _query: &'a SummaryQuery) -> ListFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn delete_run<'a>(&'a self, _run_id: Uuid) -> DeleteFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn count_history<'a>(&'a self, _query: &'a SummaryQuery) -> HistoryCountsFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn get_runs<'a>(&'a self, _run_ids: &'a [Uuid]) -> RunsFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn get_by_run_id<'a>(&'a self, _run_id: Uuid) -> GetFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn count_runs<'a>(&'a self) -> CountFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn list_trends<'a>(&'a self, _query: &'a TrendQuery) -> TrendsFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn list_projects<'a>(&'a self) -> ProjectsFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn project_totals<'a>(&'a self, _target_path: &'a str) -> ProjectTotalsFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn recent_stage_runs<'a>(&'a self, _target_path: &'a str, _limit: i64) -> StageRunsFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }

    fn ping<'a>(&'a self) -> PingFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }
}
