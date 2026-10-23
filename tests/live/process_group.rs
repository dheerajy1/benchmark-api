use std::{
    process::{Child, Command},
    thread,
    time::Duration,
};

use benchmark_api::features::v1::benchmark::process_group::{
    GroupOps, Signal, Timing, group_members, stop_child_group, terminate_verified, terminate_with,
};

use super::support::wait_until_blocking;

// Short times so the escalation is tested without waiting for the real ones
fn fast() -> Timing {
    Timing {
        term_grace: Duration::from_millis(30),
        verify_timeout: Duration::from_millis(300),
        poll: Duration::from_millis(5),
        kill_retry: Duration::from_millis(100),
    }
}

// A group made of pretend processes, it reacts to signals the way each test says
struct FakeGroup {
    members: Vec<u32>,
    clears_on: Option<Signal>,
    signals: Vec<Signal>,
}

impl FakeGroup {
    fn new(clears_on: Option<Signal>) -> Self {
        Self {
            members: vec![101, 102, 103],
            clears_on,
            signals: Vec::new(),
        }
    }
}

impl GroupOps for FakeGroup {
    fn signal(&mut self, signal: Signal) {
        self.signals.push(signal);

        if self.clears_on == Some(signal) {
            self.members.clear();
        }
    }

    fn members(&mut self) -> Vec<u32> {
        self.members.clone()
    }
}

#[test]
fn a_group_that_exits_on_term_is_not_killed() {
    let mut group = FakeGroup::new(Some(Signal::Term));

    assert_eq!(terminate_with(&mut group, &fast()), Ok(()));
    assert_eq!(group.signals, vec![Signal::Term]);
}

#[test]
fn a_group_that_ignores_term_is_killed_and_checked() {
    let mut group = FakeGroup::new(Some(Signal::Kill));

    assert_eq!(terminate_with(&mut group, &fast()), Ok(()));
    assert_eq!(group.signals, vec![Signal::Term, Signal::Kill]);
}

#[test]
fn a_group_that_survives_kill_is_an_error_that_names_what_is_left_and_kill_is_retried() {
    let mut group = FakeGroup::new(None);

    let error = terminate_with(&mut group, &fast()).unwrap_err();

    assert!(error.contains("3 process(es) still running"), "{error}");
    assert!(error.contains("101"), "{error}");
    assert_eq!(group.signals[0], Signal::Term);

    let kills = group
        .signals
        .iter()
        .filter(|signal| **signal == Signal::Kill)
        .count();

    assert!(kills >= 3, "SIGKILL was sent {kills} time(s)");
}

#[test]
fn an_empty_group_is_a_success_at_once() {
    let mut group = FakeGroup::new(None);

    group.members.clear();

    assert_eq!(terminate_with(&mut group, &fast()), Ok(()));
}

// A leader and two grandchildren in their own process group, like a benchmark target
fn spawn_group(marker: &str) -> Child {
    Command::new("setsid")
        .args([
            "sh",
            "-c",
            &format!("sleep {marker} & sleep {marker} & wait"),
        ])
        .spawn()
        .expect("setsid must be available")
}

#[test]
fn a_real_group_is_stopped_and_verified_gone_including_its_grandchildren() {
    let mut child = spawn_group("9971.5");
    let pgid = child.id();

    wait_until_blocking(|| group_members(pgid).len() >= 3, "the group to start");

    assert_eq!(terminate_verified(pgid, &Timing::default()), Ok(()));
    assert!(group_members(pgid).is_empty());

    child.wait().unwrap();
}

#[test]
fn stopping_a_child_group_collects_the_child_too() {
    let mut child = spawn_group("9972.5");
    let pgid = child.id();

    wait_until_blocking(|| group_members(pgid).len() >= 3, "the group to start");

    assert_eq!(stop_child_group(&mut child), Ok(()));
    assert!(group_members(pgid).is_empty());

    // Already collected, so there is nothing left to wait for
    assert!(child.try_wait().unwrap().is_some());
}

#[test]
fn a_process_that_ended_and_was_not_collected_is_not_counted_as_running() {
    let mut child = Command::new("setsid")
        .args(["sh", "-c", "exit 0"])
        .spawn()
        .expect("setsid must be available");

    let pgid = child.id();

    // Ended, but still a zombie because nobody called wait
    thread::sleep(Duration::from_millis(300));

    assert!(group_members(pgid).is_empty());

    child.wait().unwrap();
}

#[test]
fn a_group_that_does_not_exist_has_no_members() {
    assert!(group_members(4_000_000_000).is_empty());
}
