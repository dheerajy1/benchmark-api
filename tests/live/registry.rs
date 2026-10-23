use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use benchmark_api::features::v1::benchmark::live::{
    RunEntry, RunRegistry, event::kind, log::RunStatus, registry::MAX_TERMINAL_RUNS,
};
use serde_json::json;
use uuid::Uuid;

fn finish(entry: &RunEntry) {
    entry.append(kind::RUN_COMPLETED, json!({})).unwrap();

    // Keeps the finish times of consecutive runs strictly ordered
    thread::sleep(Duration::from_millis(2));
}

#[test]
fn a_registered_run_can_be_found_and_an_unknown_one_cannot() {
    let registry = RunRegistry::new();
    let run_id = Uuid::now_v7();

    let entry = registry.register(run_id);
    let found = registry.get(run_id).expect("registered run");

    assert!(Arc::ptr_eq(&entry, &found));
    assert_eq!(found.run_id(), run_id);
    assert_eq!(found.status(), RunStatus::Running);
    assert!(registry.get(Uuid::now_v7()).is_none());
}

#[test]
fn finished_runs_expire_after_the_retention() {
    let registry = RunRegistry::with_limits(Duration::from_secs(600), 20, 10);

    let first = registry.register(Uuid::now_v7());
    let second = registry.register(Uuid::now_v7());

    finish(&first);
    finish(&second);

    registry.evict_at(Instant::now() + Duration::from_secs(599));
    assert_eq!(registry.len(), 2);

    registry.evict_at(Instant::now() + Duration::from_secs(601));
    assert!(registry.is_empty());
}

#[test]
fn an_active_run_is_never_evicted_by_age() {
    let registry = RunRegistry::with_limits(Duration::from_secs(600), 20, 10);
    let run_id = Uuid::now_v7();

    let _active = registry.register(run_id);

    registry.evict_at(Instant::now() + Duration::from_secs(3_600));

    assert!(registry.get(run_id).is_some());
}

#[test]
fn only_the_newest_finished_runs_are_kept_when_the_cap_is_exceeded() {
    let registry = RunRegistry::with_limits(Duration::from_secs(3_600), 2, 10);

    let ids: Vec<Uuid> = (0..5).map(|_| Uuid::now_v7()).collect();

    for run_id in &ids {
        let entry = registry.register(*run_id);

        finish(&entry);
    }

    registry.evict_at(Instant::now());

    assert_eq!(registry.len(), 2);
    assert!(registry.get(ids[3]).is_some());
    assert!(registry.get(ids[4]).is_some());
    assert!(registry.get(ids[0]).is_none());
    assert!(registry.get(ids[1]).is_none());
    assert!(registry.get(ids[2]).is_none());
}

#[test]
fn an_active_run_survives_the_cap_pressure_of_finished_runs() {
    let registry = RunRegistry::with_limits(Duration::from_secs(3_600), 1, 10);
    let active_id = Uuid::now_v7();

    let _active = registry.register(active_id);

    for _ in 0..5 {
        let entry = registry.register(Uuid::now_v7());

        finish(&entry);
    }

    registry.evict_at(Instant::now());

    assert!(registry.get(active_id).is_some());
    assert_eq!(registry.len(), 2);
}

#[test]
fn the_default_limits_bound_the_registry() {
    let registry = RunRegistry::new();
    let active_id = Uuid::now_v7();

    let _active = registry.register(active_id);

    for _ in 0..(MAX_TERMINAL_RUNS + 10) {
        let entry = registry.register(Uuid::now_v7());

        finish(&entry);
    }

    registry.evict_at(Instant::now());

    assert_eq!(registry.len(), MAX_TERMINAL_RUNS + 1);
    assert!(registry.get(active_id).is_some());
}

#[test]
fn an_evicted_run_stays_readable_for_whoever_still_holds_it() {
    let registry = RunRegistry::with_limits(Duration::from_secs(1), 20, 10);
    let run_id = Uuid::now_v7();

    let entry = registry.register(run_id);

    finish(&entry);

    registry.evict_at(Instant::now() + Duration::from_secs(5));

    assert!(registry.get(run_id).is_none());
    assert_eq!(entry.replay(0).events.len(), 1);
}

#[test]
fn a_cancelled_run_expires_like_any_finished_run_and_an_active_one_does_not() {
    let registry = RunRegistry::with_limits(Duration::from_secs(600), 20, 10);

    let cancelled = registry.register(Uuid::now_v7());
    let active = registry.register(Uuid::now_v7());

    cancelled.append(kind::RUN_CANCELLING, json!({})).unwrap();
    cancelled.append(kind::RUN_CANCELLED, json!({})).unwrap();

    assert_eq!(cancelled.status(), RunStatus::Cancelled);

    registry.evict_at(Instant::now() + Duration::from_secs(601));

    assert!(registry.get(cancelled.run_id()).is_none());
    assert!(registry.get(active.run_id()).is_some());
}
