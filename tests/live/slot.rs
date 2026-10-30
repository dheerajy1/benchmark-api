use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Barrier},
    thread,
};

use benchmark_api::features::v1::benchmark::live::{ActiveRun, RunKind, RunSlot};
use uuid::Uuid;

#[test]
fn a_free_slot_can_be_acquired_and_reports_its_holder() {
    let slot = RunSlot::new();
    let run_id = Uuid::now_v7();

    assert_eq!(slot.active(), None);

    let permit = slot
        .try_acquire(RunKind::Live, Some(run_id))
        .expect("a free slot must be acquired");

    assert_eq!(permit.kind(), RunKind::Live);
    assert_eq!(permit.run_id(), Some(run_id));
    assert_eq!(
        slot.active(),
        Some(ActiveRun {
            kind: RunKind::Live,
            run_id: Some(run_id)
        })
    );
}

#[test]
fn a_second_acquire_is_refused_and_names_the_active_run() {
    let slot = RunSlot::new();
    let run_id = Uuid::now_v7();

    let _first = slot
        .try_acquire(RunKind::Live, Some(run_id))
        .expect("first acquire");

    let refused = slot
        .try_acquire(RunKind::Legacy, None)
        .err()
        .expect("the slot is taken");

    assert_eq!(
        refused,
        ActiveRun {
            kind: RunKind::Live,
            run_id: Some(run_id)
        }
    );
}

#[test]
fn a_legacy_holder_has_no_run_id() {
    let slot = RunSlot::new();

    let _permit = slot
        .try_acquire(RunKind::Legacy, None)
        .expect("first acquire");

    let refused = slot
        .try_acquire(RunKind::Live, Some(Uuid::now_v7()))
        .err()
        .expect("the slot is taken");

    assert_eq!(
        refused,
        ActiveRun {
            kind: RunKind::Legacy,
            run_id: None
        }
    );
}

#[test]
fn dropping_the_permit_releases_the_slot_for_any_kind() {
    let slot = RunSlot::new();

    let permit = slot
        .try_acquire(RunKind::Legacy, None)
        .expect("first acquire");

    drop(permit);

    assert_eq!(slot.active(), None);

    assert!(
        slot.try_acquire(RunKind::Live, Some(Uuid::now_v7()))
            .is_ok()
    );
}

#[test]
fn a_panic_while_holding_the_permit_releases_the_slot() {
    let slot = RunSlot::new();

    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let _permit = slot
            .try_acquire(RunKind::Live, None)
            .expect("first acquire");

        panic!("simulated panic while holding the permit");
    }));

    assert!(outcome.is_err());
    assert_eq!(slot.active(), None);
}

#[test]
fn racing_acquires_have_exactly_one_winner() {
    let slot = RunSlot::new();
    let barrier = Arc::new(Barrier::new(16));

    let handles: Vec<_> = (0..16)
        .map(|_| {
            let slot = Arc::clone(&slot);
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                slot.try_acquire(RunKind::Live, None)
            })
        })
        .collect();

    // The permits are kept in this vector, so the winner still holds the slot while
    // every other thread has long finished
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();

    let winners = results.iter().filter(|result| result.is_ok()).count();

    assert_eq!(winners, 1);
    assert_eq!(results.len() - winners, 15);
}
