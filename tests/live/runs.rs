use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use benchmark_api::{
    features::v1::benchmark::{
        live::{
            ActiveRun, LiveRuns, RunExecutor, RunKind, RunRegistry, RunSlot, SubmitError,
            event::kind, log::RunStatus,
        },
        model::BenchmarkResult,
    },
    persistence::{
        history::{SummaryQuery, TrendRow},
        repository::{
            BenchmarkRepository, CountFuture, GetFuture, ListFuture, PersistFuture, RunsFuture,
            TrendsFuture,
        },
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::support::{
    FailingExecutor, GatedExecutor, ImmediateExecutor, PanickingExecutor, kinds, request_for,
    test_env, wait_until_slot_free, wait_until_terminal,
};
use crate::fake_repository::{FailingRepository, InMemoryRepository};

fn live_with(repository: Arc<dyn BenchmarkRepository>, executor: Arc<dyn RunExecutor>) -> LiveRuns {
    LiveRuns::with_parts(
        test_env(),
        repository,
        RunSlot::new(),
        RunRegistry::new(),
        executor,
    )
}

// Records, at the moment of the save, whether the run was already reported completed
struct ProbeRepository {
    registry: Arc<RunRegistry>,
    saved: Mutex<Vec<Uuid>>,
    completed_visible_during_save: Mutex<Vec<bool>>,
}

impl ProbeRepository {
    fn new(registry: Arc<RunRegistry>) -> Self {
        Self {
            registry,
            saved: Mutex::new(Vec::new()),
            completed_visible_during_save: Mutex::new(Vec::new()),
        }
    }
}

impl BenchmarkRepository for ProbeRepository {
    fn save<'a>(&'a self, result: &'a BenchmarkResult) -> PersistFuture<'a> {
        Box::pin(async move {
            let visible = self
                .registry
                .get(result.run_id)
                .map(|entry| {
                    entry
                        .replay(0)
                        .events
                        .iter()
                        .any(|event| event.kind == kind::RUN_COMPLETED)
                })
                .unwrap_or(false);

            self.completed_visible_during_save
                .lock()
                .unwrap()
                .push(visible);
            self.saved.lock().unwrap().push(result.run_id);

            Ok(())
        })
    }

    fn list_summaries<'a>(&'a self, _query: &'a SummaryQuery) -> ListFuture<'a> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn get_runs<'a>(&'a self, _run_ids: &'a [Uuid]) -> RunsFuture<'a> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn get_by_run_id<'a>(&'a self, _run_id: Uuid) -> GetFuture<'a> {
        Box::pin(async { Ok(None) })
    }

    fn count_runs<'a>(&'a self) -> CountFuture<'a> {
        Box::pin(async { Ok(0) })
    }

    fn list_trends<'a>(&'a self, _limit: i64) -> TrendsFuture<'a> {
        Box::pin(async { Ok(Vec::<TrendRow>::new()) })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_persisted_before_it_is_reported_completed() {
    let temp_dir = TempDir::new().unwrap();
    let registry = RunRegistry::new();
    let probe = Arc::new(ProbeRepository::new(Arc::clone(&registry)));

    let live = LiveRuns::with_parts(
        test_env(),
        probe.clone(),
        RunSlot::new(),
        registry,
        Arc::new(ImmediateExecutor),
    );

    let accepted = live
        .submit(request_for(temp_dir.path()))
        .ok()
        .expect("accepted");

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    assert_eq!(
        kinds(&accepted.entry),
        vec![
            "run.accepted",
            "stage.started",
            "stage.completed",
            "stage.started",
            "stage.completed",
            "run.completed",
        ]
    );

    assert_eq!(accepted.entry.status(), RunStatus::Completed);
    assert_eq!(*probe.saved.lock().unwrap(), vec![accepted.run_id]);
    assert_eq!(
        *probe.completed_visible_during_save.lock().unwrap(),
        vec![false],
        "run.completed was visible before the result was saved"
    );

    let replay = accepted.entry.replay(0);
    let completed = replay.events.last().unwrap();

    assert_eq!(completed.kind, kind::RUN_COMPLETED);
    assert_eq!(completed.data["persisted"], json!(true));
    assert_eq!(
        completed.data["result"]["run_id"],
        json!(accepted.run_id.to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_run_id_exists_before_the_run_finishes_and_the_first_event_is_run_accepted() {
    let temp_dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let live = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(GatedExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let accepted = live
        .submit(request_for(temp_dir.path()))
        .ok()
        .expect("accepted");

    let from_registry = live
        .registry()
        .get(accepted.run_id)
        .expect("registered at submission");

    let replay = from_registry.replay(0);

    assert_eq!(replay.status, RunStatus::Running);
    assert_eq!(replay.events[0].seq, 1);
    assert_eq!(replay.events[0].kind, kind::RUN_ACCEPTED);
    assert_eq!(
        replay.events[0].data["capabilities"]["cancel"],
        json!(false)
    );
    assert_eq!(
        replay.events[0].data["plan"],
        json!({ "startup": false, "workload": null })
    );

    gate.store(true, Ordering::SeqCst);

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_execution_error_ends_in_run_failed_and_stores_nothing() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());

    let live = live_with(repository.clone(), Arc::new(FailingExecutor));

    let accepted = live
        .submit(request_for(temp_dir.path()))
        .ok()
        .expect("accepted");

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    let replay = accepted.entry.replay(0);
    let failed = replay.events.last().unwrap();

    assert_eq!(replay.status, RunStatus::Failed);
    assert_eq!(failed.kind, kind::RUN_FAILED);
    assert_eq!(failed.data["code"], json!("INTERNAL_SERVER_ERROR"));
    assert_eq!(failed.data["message"], json!("boom"));
    assert_eq!(failed.data["stage"], json!("build"));
    assert_eq!(failed.data["persisted"], json!(false));
    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_execution_ends_in_run_failed_and_releases_the_slot() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());

    let live = live_with(repository.clone(), Arc::new(PanickingExecutor));

    let accepted = live
        .submit(request_for(temp_dir.path()))
        .ok()
        .expect("accepted");

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    let replay = accepted.entry.replay(0);
    let failed = replay.events.last().unwrap();

    assert_eq!(replay.status, RunStatus::Failed);
    assert_eq!(failed.kind, kind::RUN_FAILED);
    assert_eq!(failed.data["stage"], json!("startup"));
    assert_eq!(failed.data["persisted"], json!(false));

    let message = failed.data["message"].as_str().unwrap();

    assert!(message.contains("panicked"), "{message}");
    assert!(message.contains("executor exploded"), "{message}");
    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_persistence_failure_ends_in_run_failed_and_never_reports_completed() {
    let temp_dir = TempDir::new().unwrap();

    let live = live_with(Arc::new(FailingRepository), Arc::new(ImmediateExecutor));

    let accepted = live
        .submit(request_for(temp_dir.path()))
        .ok()
        .expect("accepted");

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    let replay = accepted.entry.replay(0);
    let failed = replay.events.last().unwrap();

    assert_eq!(replay.status, RunStatus::Failed);
    assert_eq!(failed.kind, kind::RUN_FAILED);
    assert_eq!(failed.data["stage"], json!("saving"));
    assert_eq!(failed.data["persisted"], json!(false));
    assert!(
        failed.data["message"]
            .as_str()
            .unwrap()
            .contains("Failed to persist benchmark result")
    );
    assert!(
        !kinds(&accepted.entry).contains(&kind::RUN_COMPLETED),
        "run.completed must never be emitted when the save failed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_submission_is_refused_while_the_run_executes_and_accepted_after() {
    let temp_dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let live = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(GatedExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let first = live
        .submit(request_for(temp_dir.path()))
        .ok()
        .expect("accepted");

    assert_eq!(
        live.slot().active(),
        Some(ActiveRun {
            kind: RunKind::Live,
            run_id: Some(first.run_id)
        })
    );

    let refused = live
        .submit(request_for(temp_dir.path()))
        .err()
        .expect("refused while the first run executes");

    assert_eq!(
        refused,
        SubmitError::Busy(ActiveRun {
            kind: RunKind::Live,
            run_id: Some(first.run_id)
        })
    );

    // The refused submission created no run
    assert_eq!(live.registry().len(), 1);

    // Still held after some time, nothing but the end of the work releases it
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(live.slot().active().is_some());

    gate.store(true, Ordering::SeqCst);

    wait_until_terminal(&live, &first.entry).await;
    wait_until_slot_free(live.slot()).await;

    let third = live
        .submit(request_for(temp_dir.path()))
        .ok()
        .expect("accepted after the first run ended");

    wait_until_terminal(&live, &third.entry).await;
    wait_until_slot_free(live.slot()).await;

    assert_ne!(first.run_id, third.run_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_request_creates_no_run_and_does_not_take_the_slot() {
    let live = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(ImmediateExecutor),
    );

    let missing = std::path::Path::new("/definitely/not/a/real/directory");

    let refused = live
        .submit(request_for(missing))
        .err()
        .expect("an invalid path is rejected");

    assert!(matches!(refused, SubmitError::Invalid(_)), "{refused:?}");
    assert!(live.slot().active().is_none());
    assert!(live.registry().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_run_is_replayable_in_full_and_from_any_sequence() {
    let temp_dir = TempDir::new().unwrap();

    let live = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(ImmediateExecutor),
    );

    let accepted = live
        .submit(request_for(temp_dir.path()))
        .ok()
        .expect("accepted");

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    let entry = live
        .registry()
        .get(accepted.run_id)
        .expect("retained after the run");

    let full = entry.replay(0);

    assert_eq!(full.events.len(), 6);

    let seqs: Vec<u64> = full.events.iter().map(|event| event.seq).collect();

    assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6]);

    let late = entry.replay(full.last_seq - 1);

    assert_eq!(late.events.len(), 1);
    assert_eq!(late.events[0].kind, kind::RUN_COMPLETED);

    let none_left = entry.replay(full.last_seq);

    assert!(none_left.events.is_empty());
    assert_eq!(none_left.status, RunStatus::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_real_run_without_startup_reports_its_stages_in_order() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());

    let live = LiveRuns::new(test_env(), repository.clone(), RunSlot::new());

    let accepted = live
        .submit(request_for(temp_dir.path()))
        .ok()
        .expect("accepted");

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    let replay = accepted.entry.replay(0);

    assert_eq!(replay.status, RunStatus::Completed, "{:?}", replay.events);

    let started: Vec<&str> = replay
        .events
        .iter()
        .filter(|event| event.kind == kind::STAGE_STARTED)
        .map(|event| event.data["stage"].as_str().unwrap())
        .collect();

    assert_eq!(
        started,
        vec!["discovery", "artifact_size", "build", "startup", "saving"]
    );

    let metrics: Vec<&str> = replay
        .events
        .iter()
        .filter(|event| event.kind == kind::MEASUREMENT_AVAILABLE)
        .map(|event| event.data["metric"].as_str().unwrap())
        .collect();

    assert_eq!(
        metrics,
        vec!["discovery", "artifact_size", "build_time", "startup_time"]
    );

    let skipped: Vec<&Value> = replay
        .events
        .iter()
        .filter(|event| event.kind == kind::STAGE_SKIPPED)
        .map(|event| &event.data)
        .collect();

    assert_eq!(
        skipped,
        vec![&json!({ "stage": "workload", "reason": "no_workload" })]
    );

    // No live sampling in this phase, so no sample events exist
    assert!(!replay.events.iter().any(|event| event.kind == kind::SAMPLE));

    let last = replay.events.last().unwrap();

    assert_eq!(last.kind, kind::RUN_COMPLETED);
    assert_eq!(repository.saved_run_ids(), vec![accepted.run_id]);
    assert_eq!(
        last.data["result"]["run_id"],
        json!(accepted.run_id.to_string())
    );
}

// A runtime that shuts down while a run executes: the run must still end terminal and
// the slot must be released once the blocking work has ended
#[test]
fn a_run_reaches_a_terminal_state_when_the_runtime_shuts_down_under_it() {
    let temp_dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let live = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(GatedExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let accepted = runtime
        .block_on(async { live.submit(request_for(temp_dir.path())) })
        .ok()
        .expect("accepted");

    let release = Arc::clone(&gate);

    // Opens the gate while the runtime is shutting down, otherwise the drop below
    // would wait for the blocking work forever
    let opener = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        release.store(true, Ordering::SeqCst);
    });

    drop(runtime);
    opener.join().unwrap();

    assert!(
        accepted.entry.is_terminal(),
        "the run was left running after the runtime shut down"
    );
    assert!(live.slot().active().is_none());
}
