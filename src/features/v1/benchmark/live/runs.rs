use std::{
    any::Any,
    fs::File,
    io::Read,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    config::env::Env,
    errors::{api_error::ApiError, catalog::ErrorCode},
    features::v1::benchmark::{
        cleanup,
        configuration::RunConfiguration,
        error::BenchmarkError,
        model::{BenchmarkRequest, BenchmarkResult},
        run_origin::RunOrigin,
        service::{self, RunEnd, RunReport},
    },
    persistence::{outcome::RunOutcome, repository::BenchmarkRepository},
};

use super::{
    error::SubmitError,
    event::{format_timestamp, kind},
    registry::{RunEntry, RunRegistry},
    sink::{EntrySink, RunContext, Stage},
    slot::{RunKind, RunPermit, RunSlot},
    sockets::{SocketLimits, WsConfig},
};

// Runs the benchmark on a blocking thread. It takes the permit by reference, so it
// cannot be called without owning the execution slot. Tests replace it to exercise
// failures and panics without starting a real target.
pub trait RunExecutor: Send + Sync {
    fn execute(
        &self,
        request: &BenchmarkRequest,
        target: &Path,
        env: &Env,
        run_id: Uuid,
        ctx: &RunContext,
        permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError>;

    // The same run, keeping what finished when it fails or is cancelled. The default wraps
    // `execute`: an executor that only reports an error leaves nothing to keep, so a failed
    // or cancelled run is stored without parts.
    fn execute_report(
        &self,
        request: &BenchmarkRequest,
        target: &Path,
        env: &Env,
        run_id: Uuid,
        ctx: &RunContext,
        permit: &RunPermit,
    ) -> RunReport {
        match self.execute(request, target, env, run_id, ctx, permit) {
            Ok(result) => RunReport {
                result,
                end: RunEnd::Completed,
            },
            Err(BenchmarkError::Cancelled) => RunReport {
                result: BenchmarkResult::empty(run_id, target.display().to_string()),
                end: RunEnd::Cancelled,
            },
            Err(error) => RunReport {
                result: BenchmarkResult::empty(run_id, target.display().to_string()),
                end: RunEnd::Failed(error),
            },
        }
    }
}

// The real benchmark
pub struct ServiceExecutor;

impl RunExecutor for ServiceExecutor {
    fn execute(
        &self,
        request: &BenchmarkRequest,
        target: &Path,
        env: &Env,
        run_id: Uuid,
        ctx: &RunContext,
        permit: &RunPermit,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        service::execute(target, request, env, run_id, ctx, permit)
    }

    fn execute_report(
        &self,
        request: &BenchmarkRequest,
        target: &Path,
        env: &Env,
        run_id: Uuid,
        ctx: &RunContext,
        permit: &RunPermit,
    ) -> RunReport {
        service::execute_report(target, request, env, run_id, ctx, permit)
    }
}

// What a successful submission returns, the run id exists before the run starts
pub struct Accepted {
    pub run_id: Uuid,
    pub entry: Arc<RunEntry>,
    // The right to cancel this run, shown once to whoever started it
    pub cancel_token: String,
}

// The live submission path: validates, takes the execution slot, registers the run and
// hands it to a supervisor task that owns it until it is terminal
#[derive(Clone)]
pub struct LiveRuns {
    slot: Arc<RunSlot>,
    registry: Arc<RunRegistry>,
    repository: Arc<dyn BenchmarkRepository>,
    env: Env,
    executor: Arc<dyn RunExecutor>,
    ws: WsConfig,
    sockets: Arc<SocketLimits>,
}

impl LiveRuns {
    pub fn new(env: Env, repository: Arc<dyn BenchmarkRepository>, slot: Arc<RunSlot>) -> Self {
        Self::with_parts(
            env,
            repository,
            slot,
            RunRegistry::new(),
            Arc::new(ServiceExecutor),
        )
    }

    pub fn with_parts(
        env: Env,
        repository: Arc<dyn BenchmarkRepository>,
        slot: Arc<RunSlot>,
        registry: Arc<RunRegistry>,
        executor: Arc<dyn RunExecutor>,
    ) -> Self {
        let ws = WsConfig::default();

        Self {
            slot,
            registry,
            repository,
            env,
            executor,
            sockets: SocketLimits::new(&ws),
            ws,
        }
    }

    // Replaces the WebSocket settings, the socket counters start from zero
    pub fn with_ws_config(mut self, ws: WsConfig) -> Self {
        self.sockets = SocketLimits::new(&ws);
        self.ws = ws;

        self
    }

    pub fn ws_config(&self) -> WsConfig {
        self.ws
    }

    pub fn sockets(&self) -> &Arc<SocketLimits> {
        &self.sockets
    }

    pub fn slot(&self) -> &Arc<RunSlot> {
        &self.slot
    }

    // The status of a run that is still in the registry
    pub fn registry_status(&self, run_id: Uuid) -> Option<&'static str> {
        self.registry
            .get(run_id)
            .map(|entry| entry.status().as_str())
    }

    pub fn registry(&self) -> &Arc<RunRegistry> {
        &self.registry
    }

    // Must be called inside a tokio runtime, the supervisor is spawned on it.
    //
    // Order: validate (holds nothing), acquire the slot, register the run, spawn.
    // After the slot is acquired nothing below can fail, so a taken slot always
    // belongs to a supervisor.
    pub fn submit(&self, request: BenchmarkRequest) -> Result<Accepted, SubmitError> {
        self.submit_with_origin(request, RunOrigin::Custom)
    }

    // The same submission, recording where the run came from in run.accepted
    pub fn submit_with_origin(
        &self,
        request: BenchmarkRequest,
        origin: RunOrigin,
    ) -> Result<Accepted, SubmitError> {
        let target = service::prepare(&request).map_err(SubmitError::Invalid)?;

        let run_id = Uuid::now_v7();
        let cancel_token = new_cancel_token().map_err(SubmitError::Internal)?;

        let permit = self
            .slot
            .try_acquire(RunKind::Live, run_id)
            .map_err(SubmitError::Busy)?;

        let entry = self
            .registry
            .register_with_token(run_id, cancel_token.clone());

        let (source, target_id, target_name) = origin.event_fields();

        let _ = entry.append(
            kind::RUN_ACCEPTED,
            json!({
                "source": source,
                "target": {
                    "path": target.display().to_string(),
                    "id": target_id,
                    "name": target_name
                },
                "created_at": format_timestamp(SystemTime::now()),
                "plan": plan_of(&request),
                "capabilities": { "cancel": true },
            }),
        );

        let ownership = RunOwnership {
            entry: Arc::clone(&entry),
            permit: Some(permit),
        };

        // Captured now, while the request and its origin are still available
        let configuration = RunConfiguration::from_request(&request, &origin);

        let supervisor = Supervisor {
            repository: Arc::clone(&self.repository),
            env: self.env.clone(),
            executor: Arc::clone(&self.executor),
            run_id,
            configuration,
        };

        tokio::spawn(supervisor.run(request, target, ownership));

        Ok(Accepted {
            run_id,
            entry,
            cancel_token,
        })
    }
}

// 128 random bits from the operating system, as 32 hex digits
fn new_cancel_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];

    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| format!("Could not create a cancel token: {error}"))?;

    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn plan_of(request: &BenchmarkRequest) -> Value {
    let workload = request.workload.as_ref().map(|workload| {
        match (workload.requests, workload.duration_seconds) {
            (Some(requests), _) => json!({
                "mode": "requests",
                "requests": requests,
                "concurrency": workload.concurrency,
            }),
            (None, Some(seconds)) => json!({
                "mode": "duration",
                "duration_seconds": seconds,
                "concurrency": workload.concurrency,
            }),
            (None, None) => Value::Null,
        }
    });

    json!({
        "startup": request.startup.is_some(),
        "workload": workload,
        "workspaces": request.workspaces.len(),
        "cleanup": cleanup::is_requested(request),
    })
}

// What the supervisor owns for the whole life of a run: the run's log and the slot.
//
// Dropping it first records a terminal event when none exists, then releases the slot
// (a field is dropped after Drop::drop runs). So the slot is never free while its run
// is still reported as running, on every path including a stopped supervisor.
struct RunOwnership {
    entry: Arc<RunEntry>,
    permit: Option<RunPermit>,
}

impl Drop for RunOwnership {
    fn drop(&mut self) {
        if !self.entry.is_terminal() {
            fail(
                &self.entry,
                ErrorCode::InternalServerError.as_str(),
                "The run stopped before it reached a terminal state.".to_string(),
            );
        }
    }
}

struct Supervisor {
    repository: Arc<dyn BenchmarkRepository>,
    env: Env,
    executor: Arc<dyn RunExecutor>,
    run_id: Uuid,
    configuration: RunConfiguration,
}

// How a run that did not complete ended
enum Stopped {
    Cancelled,
    Failed { code: &'static str, message: String },
}

impl Supervisor {
    // Independent of any HTTP request: nothing a client does can stop it.
    //
    // The permit moves into the blocking closure and comes back with the outcome, so
    // it is held for the whole execution. A panic is caught inside the closure for the
    // same reason: the supervisor still holds the permit when it records the failure.
    async fn run(self, request: BenchmarkRequest, target: PathBuf, mut ownership: RunOwnership) {
        let entry = Arc::clone(&ownership.entry);
        // The execution shares the control of the run, so a cancel request reaches it
        let ctx = RunContext::with_control(
            Arc::new(EntrySink::new(Arc::clone(&entry))),
            entry.control().clone(),
        );

        let Some(permit) = ownership.permit.take() else {
            return;
        };

        let executor = Arc::clone(&self.executor);
        let env = self.env.clone();
        let run_id = self.run_id;
        let worker_ctx = ctx.clone();
        let target_text = target.display().to_string();

        let joined = tokio::task::spawn_blocking(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                executor.execute_report(&request, &target, &env, run_id, &worker_ctx, &permit)
            }));

            (outcome, permit)
        })
        .await;

        let report = match joined {
            Err(join_error) => RunReport {
                result: BenchmarkResult::empty(run_id, target_text),
                end: RunEnd::Failed(BenchmarkError::Internal(format!(
                    "Benchmark task failed: {join_error}"
                ))),
            },
            Ok((outcome, permit)) => {
                ownership.permit = Some(permit);

                match outcome {
                    Err(payload) => RunReport {
                        result: BenchmarkResult::empty(run_id, target_text),
                        end: RunEnd::Failed(BenchmarkError::Internal(format!(
                            "The benchmark panicked: {}",
                            panic_message(payload.as_ref())
                        ))),
                    },
                    Ok(report) => report,
                }
            }
        };

        self.finish(&entry, &ctx, report).await;
    }

    // Stores the run however it ended, then records the terminal event
    async fn finish(&self, entry: &RunEntry, ctx: &RunContext, report: RunReport) {
        let RunReport { result, end } = report;

        match end {
            RunEnd::Completed => {
                // The benchmark finished. If a cancel request got there first, the finished
                // work is kept as a cancelled run, otherwise cancelling is refused from now on.
                if entry.control().begin_finalizing() {
                    // Taken before saving starts: the saving stage is added by the repository
                    let outcome =
                        RunOutcome::from_result(&result).with_stages(ctx.stage_records(None));

                    self.persist(entry, ctx, result, outcome).await
                } else {
                    self.persist_stopped(
                        entry,
                        ctx,
                        result,
                        entry.current_stage(),
                        Stopped::Cancelled,
                    )
                    .await
                }
            }
            RunEnd::Cancelled => {
                self.persist_stopped(
                    entry,
                    ctx,
                    result,
                    entry.current_stage(),
                    Stopped::Cancelled,
                )
                .await
            }
            RunEnd::Failed(error) => {
                // A failure that happened on its own is reported as it happened, even when a
                // cancel request arrived at the same time. Cancelling is refused from now on.
                entry.control().begin_finalizing();

                let code = ApiError::from(error.clone()).code().as_str();

                self.persist_stopped(
                    entry,
                    ctx,
                    result,
                    entry.current_stage(),
                    Stopped::Failed {
                        code,
                        message: error.to_string(),
                    },
                )
                .await
            }
        }
    }

    // The run is only reported as completed once it is stored in one transaction
    async fn persist(
        &self,
        entry: &RunEntry,
        ctx: &RunContext,
        result: BenchmarkResult,
        outcome: RunOutcome,
    ) {
        ctx.stage_started(Stage::Saving);

        match self
            .repository
            .save_run(&result, &self.configuration, outcome)
            .await
        {
            Ok(()) => {
                ctx.stage_completed(Stage::Saving, "success", None);

                let value = serde_json::to_value(&result).unwrap_or(Value::Null);

                let _ = entry.append(
                    kind::RUN_COMPLETED,
                    json!({ "persisted": true, "result": value }),
                );
            }
            Err(error) => fail(
                entry,
                ErrorCode::InternalServerError.as_str(),
                format!("Failed to persist benchmark result: {error}"),
            ),
        }
    }

    // A failed or cancelled run is stored with every part that finished, and the stage where
    // it stopped. If storing fails the event says so with persisted false.
    async fn persist_stopped(
        &self,
        entry: &RunEntry,
        ctx: &RunContext,
        result: BenchmarkResult,
        stage: Option<String>,
        stopped: Stopped,
    ) {
        let outcome = match &stopped {
            Stopped::Cancelled => {
                RunOutcome::cancelled(stage.clone(), ctx.stage_records(Some("cancelled")))
            }
            Stopped::Failed { message, .. } => RunOutcome::failed(
                stage.clone(),
                message.clone(),
                ctx.stage_records(Some("failed")),
            ),
        };

        let saved = self
            .repository
            .save_run(&result, &self.configuration, outcome)
            .await;

        let persisted = saved.is_ok();

        let persist_error = saved
            .err()
            .map(|error| format!("Failed to persist benchmark result: {error}"));

        match stopped {
            Stopped::Cancelled => cancelled(entry, stage, persisted, persist_error),
            Stopped::Failed { code, message } => {
                fail_stored(entry, code, message, stage, persisted, persist_error)
            }
        }
    }
}

// Records the terminal failure of a run that could not be stored
fn fail(entry: &RunEntry, code: &str, message: String) {
    fail_stored(entry, code, message, entry.current_stage(), false, None);
}

// Records the terminal failure of a run, with the stage that was last started
fn fail_stored(
    entry: &RunEntry,
    code: &str,
    message: String,
    stage: Option<String>,
    persisted: bool,
    persist_error: Option<String>,
) {
    let mut data = json!({
        "code": code,
        "message": message,
        "stage": stage,
        "persisted": persisted,
    });

    if let Some(error) = persist_error {
        data["persist_error"] = json!(error);
    }

    let _ = entry.append(kind::RUN_FAILED, data);
}

// Records that the run stopped on request. Only called once the execution has returned, so its
// work and its cleanup are over.
fn cancelled(
    entry: &RunEntry,
    stage: Option<String>,
    persisted: bool,
    persist_error: Option<String>,
) {
    let mut data = json!({ "stage": stage, "persisted": persisted });

    if let Some(error) = persist_error {
        data["persist_error"] = json!(error);
    }

    let _ = entry.append(kind::RUN_CANCELLED, data);
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}
