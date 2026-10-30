use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use benchmark_api::{
    features::v1::benchmark::live::{
        CancelOutcome, CancelRequest, LiveRuns, RunControl, RunExecutor, RunRegistry, RunSlot,
        event::kind, log::RunStatus,
    },
    persistence::repository::BenchmarkRepository,
};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use super::support::{
    CancelIgnoringExecutor, CancellableExecutor, FailingExecutor, ImmediateExecutor, TimedExecutor,
    is_terminal_kind, kinds, request_for, test_env, wait_until, wait_until_slot_free,
    wait_until_terminal,
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

// ---------------------------------------------------------------------------
// The switch
// ---------------------------------------------------------------------------

#[test]
fn the_first_cancel_wins_and_a_duplicate_is_recognized() {
    let control = RunControl::default();

    assert!(!control.is_cancel_requested());
    assert_eq!(control.request_cancel(), CancelRequest::Requested);
    assert!(control.is_cancel_requested());
    assert_eq!(control.request_cancel(), CancelRequest::AlreadyRequested);

    // A run that was asked to stop does not go on to save
    assert!(!control.begin_finalizing());
}

#[test]
fn saving_closes_the_door_to_cancel() {
    let control = RunControl::default();

    assert!(control.begin_finalizing());
    assert!(!control.is_cancel_requested());
    assert_eq!(control.request_cancel(), CancelRequest::Finalizing);
    assert_eq!(control.request_cancel(), CancelRequest::Finalizing);
    assert!(!control.begin_finalizing());
}

#[test]
fn concurrent_cancels_have_exactly_one_winner() {
    let control = RunControl::default();
    let barrier = Arc::new(Barrier::new(32));

    let handles: Vec<_> = (0..32)
        .map(|_| {
            let control = control.clone();
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                control.request_cancel()
            })
        })
        .collect();

    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == CancelRequest::Requested)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == CancelRequest::AlreadyRequested)
            .count(),
        31
    );
}

#[test]
fn cancelling_and_finishing_never_both_win() {
    for _ in 0..500 {
        let control = RunControl::default();
        let barrier = Arc::new(Barrier::new(2));

        let canceller = {
            let control = control.clone();
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                control.request_cancel()
            })
        };

        let finisher = {
            let control = control.clone();
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                control.begin_finalizing()
            })
        };

        let cancel = canceller.join().unwrap();
        let finalizing = finisher.join().unwrap();

        let cancel_won = cancel == CancelRequest::Requested;

        assert!(cancel_won != finalizing, "{cancel:?} and {finalizing}");
    }
}

// ---------------------------------------------------------------------------
// One run's entry: events, tokens
// ---------------------------------------------------------------------------

#[test]
fn a_cancel_request_records_one_event_with_the_stage_and_a_duplicate_adds_none() {
    let registry = RunRegistry::new();
    let entry = registry.register_with_token(Uuid::now_v7(), "token".to_string());

    entry
        .append(kind::STAGE_STARTED, json!({ "stage": "build" }))
        .unwrap();

    assert_eq!(entry.request_cancel(), CancelOutcome::Requested);
    assert_eq!(entry.request_cancel(), CancelOutcome::AlreadyRequested);

    let events = entry.replay(0).events;
    let names: Vec<&str> = events.iter().map(|event| event.kind).collect();

    assert_eq!(names, vec!["stage.started", "run.cancelling"]);
    assert_eq!(events[1].data, json!({ "stage": "build" }));
    assert_eq!(entry.status(), RunStatus::Running);
}

#[test]
fn a_finished_run_cannot_be_cancelled_and_nothing_is_recorded() {
    for (terminal, expected) in [
        (kind::RUN_COMPLETED, RunStatus::Completed),
        (kind::RUN_FAILED, RunStatus::Failed),
        (kind::RUN_CANCELLED, RunStatus::Cancelled),
    ] {
        let registry = RunRegistry::new();
        let entry = registry.register(Uuid::now_v7());

        entry.append(terminal, json!({})).unwrap();

        assert_eq!(entry.request_cancel(), CancelOutcome::Finished(expected));
        assert_eq!(entry.replay(0).events.len(), 1);
        assert!(!entry.control().is_cancel_requested());
    }
}

#[test]
fn a_run_that_is_saving_cannot_be_cancelled() {
    let registry = RunRegistry::new();
    let entry = registry.register(Uuid::now_v7());

    assert!(entry.control().begin_finalizing());
    assert_eq!(entry.request_cancel(), CancelOutcome::Saving);
    assert!(entry.replay(0).events.is_empty());
}

#[test]
fn only_the_exact_token_is_accepted() {
    let registry = RunRegistry::new();
    let entry = registry.register_with_token(Uuid::now_v7(), "0123456789abcdef".to_string());

    assert!(entry.verify_token("0123456789abcdef"));

    for wrong in [
        "",
        "0123456789abcdee",
        "0123456789abcde",
        "0123456789abcdef0",
        "0123456789ABCDEF",
        " 0123456789abcdef",
    ] {
        assert!(!entry.verify_token(wrong), "{wrong:?}");
    }
}

#[test]
fn a_run_registered_without_a_token_can_never_be_cancelled_by_token() {
    let registry = RunRegistry::new();
    let entry = registry.register(Uuid::now_v7());

    assert!(!entry.verify_token(""));
    assert!(!entry.verify_token("anything"));
}

// ---------------------------------------------------------------------------
// The supervisor
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_submission_gets_a_random_token_that_is_never_part_of_an_event() {
    let temp_dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let live = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let first = live.submit(request_for(temp_dir.path())).expect("accepted");

    assert_eq!(first.cancel_token.len(), 32);
    assert!(first.cancel_token.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(first.entry.verify_token(&first.cancel_token));
    assert!(
        !first
            .cancel_token
            .contains(&first.run_id.simple().to_string())
    );

    gate.store(true, Ordering::SeqCst);
    wait_until_terminal(&live, &first.entry).await;
    wait_until_slot_free(live.slot()).await;

    let second = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until_terminal(&live, &second.entry).await;
    wait_until_slot_free(live.slot()).await;

    assert_ne!(first.cancel_token, second.cancel_token);

    // Neither token appears anywhere in what a client can read
    for accepted in [&first, &second] {
        let text = serde_json::to_string(&accepted.entry.replay(0).events).unwrap();

        assert!(!text.contains(&first.cancel_token));
        assert!(!text.contains(&second.cancel_token));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_run_that_works_ends_in_exactly_one_run_cancelled_and_stores_nothing() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());
    let gate = Arc::new(AtomicBool::new(false));

    let live = live_with(
        repository.clone(),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let accepted = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until(
        || kinds(&accepted.entry).contains(&kind::STAGE_STARTED),
        "the build to start",
    )
    .await;

    assert_eq!(accepted.entry.request_cancel(), CancelOutcome::Requested);

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    assert_eq!(
        kinds(&accepted.entry),
        vec![
            "run.accepted",
            "stage.started",
            "run.cancelling",
            "run.cancelled"
        ]
    );

    let replay = accepted.entry.replay(0);
    let last = replay.events.last().unwrap();

    assert_eq!(replay.status, RunStatus::Cancelled);
    assert_eq!(last.data["stage"], json!("build"));
    assert_eq!(last.data["persisted"], json!(false));
    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_is_not_reported_until_the_work_has_stopped() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());
    let gate = Arc::new(AtomicBool::new(false));

    // This execution ignores the request, it only stops when its gate opens
    let live = live_with(
        repository.clone(),
        Arc::new(CancelIgnoringExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let accepted = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until(
        || kinds(&accepted.entry).contains(&kind::STAGE_STARTED),
        "the build to start",
    )
    .await;

    assert_eq!(accepted.entry.request_cancel(), CancelOutcome::Requested);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // The run is stopping, not stopped: still running, slot still held, no terminal event
    assert_eq!(accepted.entry.status(), RunStatus::Running);
    assert!(kinds(&accepted.entry).contains(&kind::RUN_CANCELLING));
    assert!(!kinds(&accepted.entry).iter().any(|k| is_terminal_kind(k)));
    assert!(live.slot().active().is_some());

    gate.store(true, Ordering::SeqCst);

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    // The benchmark ran to its end while a cancel request was pending: the result is thrown
    // away and nothing is saved
    assert_eq!(accepted.entry.status(), RunStatus::Cancelled);
    assert!(repository.saved_run_ids().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_run_never_reaches_the_repository_not_even_a_failing_one() {
    let temp_dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let live = live_with(
        Arc::new(FailingRepository),
        Arc::new(CancelIgnoringExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let accepted = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until(
        || kinds(&accepted.entry).contains(&kind::STAGE_STARTED),
        "the build to start",
    )
    .await;

    accepted.entry.request_cancel();
    gate.store(true, Ordering::SeqCst);

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    // A save attempt would have ended this run as failed
    assert_eq!(accepted.entry.status(), RunStatus::Cancelled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_started_saving_is_completed_and_cancelling_is_refused() {
    use benchmark_api::{
        features::v1::benchmark::model::BenchmarkResult, persistence::repository::PersistFuture,
    };

    // Saving takes a while, so there is a window to try cancelling in
    struct SlowRepository {
        saved: std::sync::Mutex<Vec<Uuid>>,
    }

    impl BenchmarkRepository for SlowRepository {
        fn save<'a>(
            &'a self,
            result: &'a BenchmarkResult,
            _configuration: &'a benchmark_api::features::v1::benchmark::configuration::RunConfiguration,
        ) -> PersistFuture<'a> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(400)).await;

                self.saved.lock().unwrap().push(result.run_id);

                Ok(())
            })
        }

        fn get_configuration<'a>(
            &'a self,
            _run_id: Uuid,
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

        fn list_summaries<'a>(
            &'a self,
            _query: &'a benchmark_api::persistence::history::SummaryQuery,
        ) -> benchmark_api::persistence::repository::ListFuture<'a> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn get_runs<'a>(
            &'a self,
            _run_ids: &'a [Uuid],
        ) -> benchmark_api::persistence::repository::RunsFuture<'a> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn get_by_run_id<'a>(
            &'a self,
            _run_id: Uuid,
        ) -> benchmark_api::persistence::repository::GetFuture<'a> {
            Box::pin(async { Ok(None) })
        }

        fn count_runs<'a>(&'a self) -> benchmark_api::persistence::repository::CountFuture<'a> {
            Box::pin(async { Ok(0) })
        }

        fn list_trends<'a>(
            &'a self,
            _limit: i64,
        ) -> benchmark_api::persistence::repository::TrendsFuture<'a> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(SlowRepository {
        saved: std::sync::Mutex::new(Vec::new()),
    });

    let live = live_with(repository.clone(), Arc::new(ImmediateExecutor));

    let accepted = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until(
        || {
            accepted
                .entry
                .replay(0)
                .events
                .iter()
                .any(|event| event.data["stage"] == json!("saving"))
        },
        "the run to start saving",
    )
    .await;

    assert_eq!(accepted.entry.request_cancel(), CancelOutcome::Saving);

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    assert_eq!(accepted.entry.status(), RunStatus::Completed);
    assert!(!kinds(&accepted.entry).contains(&kind::RUN_CANCELLING));
    assert_eq!(*repository.saved.lock().unwrap(), vec![accepted.run_id]);
}

#[tokio::test(flavor = "multi_thread")]
async fn sixteen_concurrent_cancels_record_one_request_and_one_end() {
    let temp_dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    let live = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let accepted = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until(
        || kinds(&accepted.entry).contains(&kind::STAGE_STARTED),
        "the build to start",
    )
    .await;

    let barrier = Arc::new(Barrier::new(16));

    let handles: Vec<_> = (0..16)
        .map(|_| {
            let entry = Arc::clone(&accepted.entry);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                entry.request_cancel()
            })
        })
        .collect();

    let outcomes: Vec<CancelOutcome> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == CancelOutcome::Requested)
            .count(),
        1
    );

    wait_until_terminal(&live, &accepted.entry).await;
    wait_until_slot_free(live.slot()).await;

    let names = kinds(&accepted.entry);

    assert_eq!(names.iter().filter(|k| **k == "run.cancelling").count(), 1);
    assert_eq!(names.iter().filter(|k| is_terminal_kind(k)).count(), 1);
    assert_eq!(accepted.entry.status(), RunStatus::Cancelled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_run_answers_every_cancel_with_its_status_and_changes_nothing() {
    let temp_dir = TempDir::new().unwrap();
    let gate = Arc::new(AtomicBool::new(false));

    // One run of each ending, one after the other
    let completed = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(ImmediateExecutor),
    );
    let failed = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(FailingExecutor),
    );
    let cancelled = live_with(
        Arc::new(InMemoryRepository::default()),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let a = completed
        .submit(request_for(temp_dir.path()))
        .expect("accepted");
    wait_until_terminal(&completed, &a.entry).await;
    wait_until_slot_free(completed.slot()).await;

    let b = failed
        .submit(request_for(temp_dir.path()))
        .expect("accepted");
    wait_until_terminal(&failed, &b.entry).await;
    wait_until_slot_free(failed.slot()).await;

    let c = cancelled
        .submit(request_for(temp_dir.path()))
        .expect("accepted");
    wait_until(
        || kinds(&c.entry).contains(&kind::STAGE_STARTED),
        "the build",
    )
    .await;
    c.entry.request_cancel();
    wait_until_terminal(&cancelled, &c.entry).await;
    wait_until_slot_free(cancelled.slot()).await;

    for (entry, expected) in [
        (&a.entry, RunStatus::Completed),
        (&b.entry, RunStatus::Failed),
        (&c.entry, RunStatus::Cancelled),
    ] {
        let before = entry.replay(0).events.len();

        assert_eq!(entry.request_cancel(), CancelOutcome::Finished(expected));
        assert_eq!(entry.request_cancel(), CancelOutcome::Finished(expected));
        assert_eq!(entry.replay(0).events.len(), before);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_cancellation_the_slot_is_free_and_the_next_run_works() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());
    let gate = Arc::new(AtomicBool::new(false));

    let live = live_with(
        repository.clone(),
        Arc::new(CancellableExecutor {
            gate: Arc::clone(&gate),
        }),
    );

    let first = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until(
        || kinds(&first.entry).contains(&kind::STAGE_STARTED),
        "the build to start",
    )
    .await;

    first.entry.request_cancel();
    wait_until_terminal(&live, &first.entry).await;
    wait_until_slot_free(live.slot()).await;

    gate.store(true, Ordering::SeqCst);

    let second = live.submit(request_for(temp_dir.path())).expect("accepted");

    wait_until_terminal(&live, &second.entry).await;
    wait_until_slot_free(live.slot()).await;

    assert_eq!(first.entry.status(), RunStatus::Cancelled);
    assert_eq!(second.entry.status(), RunStatus::Completed);
    assert_eq!(repository.saved_run_ids(), vec![second.run_id]);
    assert_eq!(live.registry().len(), 2);
}

// ---------------------------------------------------------------------------
// Races
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_a_run_completes_always_ends_in_exactly_one_consistent_terminal_event() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());

    let live = live_with(
        repository.clone(),
        Arc::new(TimedExecutor {
            delay: Duration::from_millis(3),
            fail: false,
        }),
    );

    let mut completed = 0;
    let mut cancelled = 0;

    for iteration in 0..80u64 {
        wait_until_slot_free(live.slot()).await;

        let accepted = live.submit(request_for(temp_dir.path())).expect("accepted");

        // Spreads the cancel request over the moment the run finishes
        tokio::time::sleep(Duration::from_micros((iteration % 16) * 250)).await;

        let outcome = accepted.entry.request_cancel();

        wait_until_terminal(&live, &accepted.entry).await;

        let names = kinds(&accepted.entry);
        let terminals: Vec<&str> = names
            .iter()
            .copied()
            .filter(|name| is_terminal_kind(name))
            .collect();

        assert_eq!(terminals.len(), 1, "{names:?}");

        let saved_times = repository
            .saved_run_ids()
            .iter()
            .filter(|id| **id == accepted.run_id)
            .count();

        match terminals[0] {
            "run.completed" => {
                completed += 1;

                assert_eq!(saved_times, 1);
                assert_ne!(outcome, CancelOutcome::Requested, "{names:?}");
            }
            "run.cancelled" => {
                cancelled += 1;

                assert_eq!(saved_times, 0);
                assert_eq!(outcome, CancelOutcome::Requested, "{names:?}");
            }
            other => panic!("unexpected end {other}: {names:?}"),
        }
    }

    wait_until_slot_free(live.slot()).await;

    // Never saved twice, never saved when cancelled
    assert_eq!(repository.saved_run_ids().len(), completed);
    assert_eq!(completed + cancelled, 80);
    println!("PHASEC1 race completion: completed={completed} cancelled={cancelled}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_a_run_fails_ends_in_failed_because_that_is_what_happened() {
    let temp_dir = TempDir::new().unwrap();
    let repository = Arc::new(InMemoryRepository::default());

    let live = live_with(
        repository.clone(),
        Arc::new(TimedExecutor {
            delay: Duration::from_millis(3),
            fail: true,
        }),
    );

    for iteration in 0..60u64 {
        wait_until_slot_free(live.slot()).await;

        let accepted = live.submit(request_for(temp_dir.path())).expect("accepted");

        tokio::time::sleep(Duration::from_micros((iteration % 16) * 250)).await;

        accepted.entry.request_cancel();

        wait_until_terminal(&live, &accepted.entry).await;

        let names = kinds(&accepted.entry);
        let terminals: Vec<&str> = names
            .iter()
            .copied()
            .filter(|name| is_terminal_kind(name))
            .collect();

        assert_eq!(terminals, vec!["run.failed"], "{names:?}");
    }

    wait_until_slot_free(live.slot()).await;

    assert!(repository.saved_run_ids().is_empty());
}
