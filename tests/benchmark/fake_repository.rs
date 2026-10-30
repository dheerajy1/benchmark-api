use std::sync::Mutex;

use benchmark_api::{
    features::v1::benchmark::{metrics::http_latency::STATUS_COMPLETED, model::BenchmarkResult},
    persistence::{
        error::PersistenceError,
        history::{SummaryQuery, SummaryRow, TrendRow},
        postgres::mapping::{STATUS_SUCCESS, run_created_at},
        repository::{
            BenchmarkRepository, CountFuture, GetFuture, ListFuture, PersistFuture, RunsFuture,
            TrendsFuture,
        },
        stored_run::StoredRun,
    },
};
use uuid::Uuid;

// The runtime codes the seed migration inserts, the fake rejects any other code like SQL does
const KNOWN_RUNTIMES: [&str; 5] = ["bun", "node", "python", "rust", "go"];

// Keeps what it is asked to save, so API tests need no database and can read it back
#[derive(Default)]
pub struct InMemoryRepository {
    saved: Mutex<Vec<Uuid>>,
    results: Mutex<Vec<BenchmarkResult>>,
    extra_summaries: Mutex<Vec<SummaryRow>>,
}

impl InMemoryRepository {
    pub fn saved_run_ids(&self) -> Vec<Uuid> {
        self.saved.lock().unwrap().clone()
    }

    // Adds a complete result without going through save, to control every metric exactly
    pub fn seed_result(&self, result: BenchmarkResult) {
        self.results.lock().unwrap().push(result);
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
        let runtime_matches = query
            .runtime_code
            .as_ref()
            .is_none_or(|code| row.runtime_code.as_ref() == Some(code));

        let path_matches = query
            .target_path
            .as_ref()
            .is_none_or(|path| &row.target_path == path);

        let after_cursor = query
            .after
            .as_ref()
            .is_none_or(|after| (row.created_at, row.id) < (after.created_at, after.id));

        runtime_matches && path_matches && after_cursor
    });

    rows.sort_by_key(|row| std::cmp::Reverse((row.created_at, row.id)));
    rows.truncate(query.limit as usize);

    Ok(rows)
}

fn summary_of(result: &BenchmarkResult) -> SummaryRow {
    let metrics = &result.metrics;

    SummaryRow {
        id: result.run_id,
        target_path: result.target.path.clone(),
        runtime_code: result.discovery.runtime.clone(),
        created_at: run_created_at(result.run_id).unwrap(),
        build_time_ms: metrics.build_time.duration_ms.map(|value| value as i64),
        startup_time_ms: metrics.startup_time.duration_ms.map(|value| value as i64),
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
fn trends(repository: &InMemoryRepository, limit: i64) -> Vec<TrendRow> {
    let mut rows: Vec<TrendRow> = repository
        .results
        .lock()
        .unwrap()
        .iter()
        .map(trend_of)
        .collect();

    rows.sort_by_key(|row| std::cmp::Reverse((row.created_at, row.id)));
    rows.truncate(usize::try_from(limit).unwrap_or(0));

    rows
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
        build_time_ms: successful_duration(
            &metrics.build_time.status,
            metrics.build_time.duration_ms,
        ),
        startup_time_ms: successful_duration(
            &metrics.startup_time.status,
            metrics.startup_time.duration_ms,
        ),
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
    fn save<'a>(
        &'a self,
        result: &'a BenchmarkResult,
        _configuration: &'a benchmark_api::features::v1::benchmark::configuration::RunConfiguration,
    ) -> PersistFuture<'a> {
        Box::pin(async move {
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
                .find(|result| result.run_id == run_id)
                .cloned())
        })
    }

    fn count_runs<'a>(&'a self) -> CountFuture<'a> {
        Box::pin(async move {
            let results = self.results.lock().unwrap().len();
            let summaries = self.extra_summaries.lock().unwrap().len();

            Ok((results + summaries) as i64)
        })
    }

    fn list_trends<'a>(&'a self, limit: i64) -> TrendsFuture<'a> {
        Box::pin(async move { Ok(trends(self, limit)) })
    }
}

impl InMemoryRepository {
    fn runs(&self, run_ids: &[Uuid]) -> Vec<StoredRun> {
        self.results
            .lock()
            .unwrap()
            .iter()
            .filter(|result| run_ids.contains(&result.run_id))
            .map(|result| StoredRun {
                created_at: run_created_at(result.run_id).unwrap(),
                result: result.clone(),
            })
            .collect()
    }
}

// Always fails, to prove the API does not report a benchmark that was not stored
pub struct FailingRepository;

impl BenchmarkRepository for FailingRepository {
    fn save<'a>(
        &'a self,
        _result: &'a BenchmarkResult,
        _configuration: &'a benchmark_api::features::v1::benchmark::configuration::RunConfiguration,
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

    fn list_trends<'a>(&'a self, _limit: i64) -> TrendsFuture<'a> {
        Box::pin(async {
            Err(PersistenceError::Database(
                "simulated persistence failure".to_string(),
            ))
        })
    }
}
