use benchmark_api::features::v1::benchmark::live::{
    event::{RunEvent, kind},
    log::{AppendError, EventLog, MAX_STRUCTURAL_EVENTS, RunStatus, SAMPLE_RING_CAPACITY},
};
use serde_json::{Value, json};
use uuid::Uuid;

fn log_with_capacity(capacity: usize) -> EventLog {
    EventLog::with_sample_capacity(Uuid::now_v7(), capacity)
}

fn add(log: &mut EventLog, event_kind: &'static str) -> u64 {
    log.append(event_kind, json!({})).expect("append")
}

fn seqs(events: &[RunEvent]) -> Vec<u64> {
    events.iter().map(|event| event.seq).collect()
}

// The documented client rule: a gap in seq is legitimate only while the last seen seq is
// below dropped_through_seq, beyond it the sequence must be contiguous
fn accepted_by_client(events: &[RunEvent], after: u64, dropped_through_seq: u64) -> bool {
    let mut last = after;

    for event in events {
        if event.seq <= last {
            return false;
        }

        if event.seq != last + 1 && last >= dropped_through_seq {
            return false;
        }

        last = event.seq;
    }

    true
}

// seq 1 accepted, 2 stage.started, 3..=12 ten samples, 13 stage.completed, 14 completed
fn overflowed_log() -> EventLog {
    let mut log = log_with_capacity(3);

    add(&mut log, kind::RUN_ACCEPTED);
    add(&mut log, kind::STAGE_STARTED);

    for _ in 0..10 {
        add(&mut log, kind::SAMPLE);
    }

    add(&mut log, kind::STAGE_COMPLETED);
    add(&mut log, kind::RUN_COMPLETED);

    log
}

#[test]
fn sequence_numbers_start_at_one_and_grow_by_one() {
    let mut log = log_with_capacity(10);

    assert_eq!(add(&mut log, kind::RUN_ACCEPTED), 1);
    assert_eq!(add(&mut log, kind::STAGE_STARTED), 2);
    assert_eq!(add(&mut log, kind::SAMPLE), 3);
    assert_eq!(add(&mut log, kind::STAGE_COMPLETED), 4);
    assert_eq!(log.last_seq(), 4);
}

#[test]
fn an_untruncated_log_replays_everything_in_order() {
    let mut log = log_with_capacity(10);

    add(&mut log, kind::RUN_ACCEPTED);
    add(&mut log, kind::SAMPLE);
    add(&mut log, kind::STAGE_STARTED);
    add(&mut log, kind::SAMPLE);

    let replay = log.replay(0);

    assert_eq!(seqs(&replay.events), vec![1, 2, 3, 4]);
    assert_eq!(replay.dropped_through_seq, 0);
    assert!(!replay.truncated());
    assert_eq!(replay.status, RunStatus::Running);
    assert!(accepted_by_client(
        &replay.events,
        0,
        replay.dropped_through_seq
    ));
}

#[test]
fn structural_events_survive_when_samples_overflow() {
    let log = overflowed_log();
    let replay = log.replay(0);

    // Samples 3..=9 were evicted, the three newest samples 10, 11 and 12 remain
    assert_eq!(seqs(&replay.events), vec![1, 2, 10, 11, 12, 13, 14]);
    assert_eq!(replay.dropped_through_seq, 9);
    assert!(replay.truncated());
    assert_eq!(replay.last_seq, 14);

    let structural: Vec<&str> = replay
        .events
        .iter()
        .map(|event| event.kind)
        .filter(|event_kind| *event_kind != kind::SAMPLE)
        .collect();

    assert_eq!(
        structural,
        vec![
            kind::RUN_ACCEPTED,
            kind::STAGE_STARTED,
            kind::STAGE_COMPLETED,
            kind::RUN_COMPLETED
        ]
    );
}

#[test]
fn the_gaps_of_a_truncated_replay_follow_the_client_rule() {
    let replay = overflowed_log().replay(0);

    assert!(accepted_by_client(
        &replay.events,
        0,
        replay.dropped_through_seq
    ));
}

#[test]
fn a_hole_beyond_the_dropped_range_breaks_the_client_rule() {
    let mut events = overflowed_log().replay(0).events;

    events.retain(|event| event.seq != 12);

    assert!(!accepted_by_client(&events, 0, 9));
}

#[test]
fn a_duplicate_breaks_the_client_rule() {
    let mut events = overflowed_log().replay(0).events;

    events.push(events[0].clone());

    assert!(!accepted_by_client(&events, 0, 9));
}

#[test]
fn replay_after_a_sequence_returns_only_later_events() {
    let log = overflowed_log();

    let replay = log.replay(10);

    assert_eq!(seqs(&replay.events), vec![11, 12, 13, 14]);
    assert_eq!(replay.dropped_through_seq, 9);
    assert!(accepted_by_client(&replay.events, 10, 9));
}

#[test]
fn replay_from_inside_the_dropped_range_starts_at_the_first_retained_event() {
    let log = overflowed_log();

    let replay = log.replay(5);

    assert_eq!(seqs(&replay.events), vec![10, 11, 12, 13, 14]);
    assert!(accepted_by_client(&replay.events, 5, 9));
}

#[test]
fn replay_after_the_last_seq_is_empty_but_reports_the_terminal_status() {
    let log = overflowed_log();

    let replay = log.replay(log.last_seq());

    assert!(replay.events.is_empty());
    assert_eq!(replay.status, RunStatus::Completed);
    assert_eq!(replay.last_seq, 14);
}

#[test]
fn a_late_replay_recovers_the_terminal_event_even_when_every_sample_was_evicted() {
    let log = overflowed_log();

    let replay = log.replay(13);

    assert_eq!(seqs(&replay.events), vec![14]);
    assert_eq!(replay.events[0].kind, kind::RUN_COMPLETED);
}

#[test]
fn the_default_ring_keeps_the_newest_twelve_hundred_samples() {
    let mut log = EventLog::new(Uuid::now_v7());

    for _ in 0..1_300 {
        add(&mut log, kind::SAMPLE);
    }

    let replay = log.replay(0);

    assert_eq!(SAMPLE_RING_CAPACITY, 1_200);
    assert_eq!(replay.events.len(), 1_200);
    assert_eq!(replay.dropped_through_seq, 100);
    assert_eq!(replay.events[0].seq, 101);
}

#[test]
fn nothing_can_be_appended_after_a_terminal_event() {
    let mut log = log_with_capacity(10);

    add(&mut log, kind::RUN_ACCEPTED);
    add(&mut log, kind::RUN_COMPLETED);

    assert_eq!(log.status(), RunStatus::Completed);
    assert!(log.is_terminal());
    assert!(log.terminal_at().is_some());

    assert_eq!(
        log.append(kind::STAGE_STARTED, json!({})),
        Err(AppendError::AlreadyTerminal)
    );
    assert_eq!(
        log.append(kind::RUN_FAILED, json!({})),
        Err(AppendError::AlreadyTerminal)
    );
    assert_eq!(
        log.append(kind::SAMPLE, json!({})),
        Err(AppendError::AlreadyTerminal)
    );
    assert_eq!(log.last_seq(), 2);
}

#[test]
fn a_failed_terminal_event_sets_the_failed_status() {
    let mut log = log_with_capacity(10);

    add(&mut log, kind::RUN_ACCEPTED);
    add(&mut log, kind::RUN_FAILED);

    assert_eq!(log.status(), RunStatus::Failed);
    assert_eq!(log.status().as_str(), "failed");
}

#[test]
fn a_running_log_has_no_terminal_time() {
    let mut log = log_with_capacity(10);

    add(&mut log, kind::RUN_ACCEPTED);

    assert_eq!(log.status(), RunStatus::Running);
    assert!(log.terminal_at().is_none());
}

#[test]
fn the_structural_bound_refuses_events_but_never_the_terminal_one() {
    let mut log = log_with_capacity(10);

    for _ in 0..(MAX_STRUCTURAL_EVENTS - 1) {
        add(&mut log, kind::STAGE_STARTED);
    }

    assert_eq!(
        log.append(kind::STAGE_STARTED, json!({})),
        Err(AppendError::StructuralLimit)
    );

    assert!(log.append(kind::RUN_FAILED, json!({})).is_ok());
    assert_eq!(log.status(), RunStatus::Failed);
}

#[test]
fn samples_do_not_count_toward_the_structural_bound() {
    let mut log = log_with_capacity(10);

    for _ in 0..(MAX_STRUCTURAL_EVENTS * 3) {
        add(&mut log, kind::SAMPLE);
    }

    assert!(log.append(kind::STAGE_STARTED, json!({})).is_ok());
}

#[test]
fn the_current_stage_is_the_last_started_stage() {
    let mut log = log_with_capacity(10);

    assert_eq!(log.current_stage(), None);

    log.append(kind::STAGE_STARTED, json!({ "stage": "build" }))
        .unwrap();
    log.append(kind::STAGE_STARTED, json!({ "stage": "startup" }))
        .unwrap();
    log.append(kind::STAGE_COMPLETED, json!({ "stage": "startup" }))
        .unwrap();

    assert_eq!(log.current_stage(), Some("startup"));
}

#[test]
fn every_event_carries_the_envelope_of_the_contract() {
    let run_id = Uuid::now_v7();
    let mut log = EventLog::with_sample_capacity(run_id, 10);

    log.append(kind::STAGE_STARTED, json!({ "stage": "build" }))
        .unwrap();

    let event = log.replay(0).events.remove(0);
    let value: Value = serde_json::to_value(&event).unwrap();

    assert_eq!(value["v"], json!(1));
    assert_eq!(value["run_id"], json!(run_id.to_string()));
    assert_eq!(value["seq"], json!(1));
    assert_eq!(value["type"], json!("stage.started"));
    assert_eq!(value["data"], json!({ "stage": "build" }));
    assert!(value["t_ms"].is_u64());

    let at = value["at"].as_str().unwrap();

    assert_eq!(at.len(), 24);
    assert!(at.ends_with('Z'));
    assert_eq!(&at[10..11], "T");
}
