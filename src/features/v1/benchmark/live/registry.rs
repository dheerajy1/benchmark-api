use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::sync::watch;
use uuid::Uuid;

use super::{
    control::{CancelRequest, RunControl},
    event::kind,
    log::{AppendError, EventLog, Replay, RunStatus, SAMPLE_RING_CAPACITY},
};

// How long a finished run stays replayable
pub const RETENTION: Duration = Duration::from_secs(600);

// Finished runs kept at most, active runs are not counted
pub const MAX_TERMINAL_RUNS: usize = 20;

// What a request to cancel a run led to
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    // This request started the cancellation and `run.cancelling` was recorded
    Requested,
    // Cancellation was already underway, nothing was added
    AlreadyRequested,
    // The run is saving its result, too late to cancel
    Saving,
    // The run already ended
    Finished(RunStatus),
}

// One run in the registry, shared by the supervisor that writes and the readers
pub struct RunEntry {
    run_id: Uuid,
    log: Mutex<EventLog>,
    control: RunControl,
    // Proves the right to cancel, kept in memory only and never part of an event
    cancel_token: String,
    // Carries the seq of the last appended event, so a reader can wait for news
    notify: watch::Sender<u64>,
    // WebSockets currently attached to this run
    sockets: AtomicUsize,
}

impl RunEntry {
    fn new(run_id: Uuid, sample_capacity: usize, cancel_token: String) -> Self {
        Self {
            run_id,
            log: Mutex::new(EventLog::with_sample_capacity(run_id, sample_capacity)),
            control: RunControl::default(),
            cancel_token,
            notify: watch::channel(0).0,
            sockets: AtomicUsize::new(0),
        }
    }

    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    // The switch shared with the execution of this run
    pub fn control(&self) -> &RunControl {
        &self.control
    }

    // Whether the presented token is the one of this run. An entry without a token, and an
    // empty token, never match. The comparison takes the same time wherever they differ.
    pub fn verify_token(&self, presented: &str) -> bool {
        !self.cancel_token.is_empty()
            && constant_time_eq(self.cancel_token.as_bytes(), presented.as_bytes())
    }

    // Asks the run to stop. Everything happens under the log lock, so a run that ends at the
    // same moment is either seen as ended here, or gets `run.cancelling` before its end event.
    // The terminal event itself is recorded by the supervisor once the work has really stopped.
    pub fn request_cancel(&self) -> CancelOutcome {
        let (outcome, appended) = {
            let mut log = lock(&self.log);

            if log.is_terminal() {
                return CancelOutcome::Finished(log.status());
            }

            match self.control.request_cancel() {
                CancelRequest::Requested => {
                    let stage = log.current_stage().map(str::to_string);
                    let appended = log
                        .append(kind::RUN_CANCELLING, json!({ "stage": stage }))
                        .ok();

                    (CancelOutcome::Requested, appended)
                }
                CancelRequest::AlreadyRequested => (CancelOutcome::AlreadyRequested, None),
                CancelRequest::Finalizing => (CancelOutcome::Saving, None),
            }
        };

        if let Some(seq) = appended {
            self.notify.send_replace(seq);
        }

        outcome
    }

    pub fn append(&self, kind: &'static str, data: Value) -> Result<u64, AppendError> {
        let appended = lock(&self.log).append(kind, data);

        // Outside the log lock, so a woken reader never waits for the writer
        if let Ok(seq) = appended {
            self.notify.send_replace(seq);
        }

        appended
    }

    // A receiver that is woken by every new event. Subscribe before reading the log,
    // then no event can be missed between the read and the wait.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.notify.subscribe()
    }

    pub fn socket_count(&self) -> usize {
        self.sockets.load(Ordering::SeqCst)
    }

    // Takes a socket place unless the run already has `max`
    pub(super) fn try_acquire_socket(&self, max: usize) -> bool {
        if self.sockets.fetch_add(1, Ordering::SeqCst) >= max {
            self.sockets.fetch_sub(1, Ordering::SeqCst);

            return false;
        }

        true
    }

    pub(super) fn release_socket(&self) {
        self.sockets.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn replay(&self, after: u64) -> Replay {
        lock(&self.log).replay(after)
    }

    pub fn status(&self) -> RunStatus {
        lock(&self.log).status()
    }

    pub fn is_terminal(&self) -> bool {
        lock(&self.log).is_terminal()
    }

    pub fn current_stage(&self) -> Option<String> {
        lock(&self.log).current_stage().map(str::to_string)
    }

    pub fn terminal_at(&self) -> Option<Instant> {
        lock(&self.log).terminal_at()
    }
}

// Every run of this process, in memory only. Finished runs expire by age and by
// count; a run that is not finished is never evicted, whatever the limits say.
pub struct RunRegistry {
    runs: Mutex<HashMap<Uuid, Arc<RunEntry>>>,
    retention: Duration,
    max_terminal: usize,
    sample_capacity: usize,
}

impl RunRegistry {
    pub fn new() -> Arc<Self> {
        Self::with_limits(RETENTION, MAX_TERMINAL_RUNS, SAMPLE_RING_CAPACITY)
    }

    pub fn with_limits(
        retention: Duration,
        max_terminal: usize,
        sample_capacity: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            runs: Mutex::new(HashMap::new()),
            retention,
            max_terminal,
            sample_capacity,
        })
    }

    // A run nobody can cancel, its token is empty and never matches
    pub fn register(&self, run_id: Uuid) -> Arc<RunEntry> {
        self.register_with_token(run_id, String::new())
    }

    pub fn register_with_token(&self, run_id: Uuid, cancel_token: String) -> Arc<RunEntry> {
        self.evict_at(Instant::now());

        let entry = Arc::new(RunEntry::new(run_id, self.sample_capacity, cancel_token));

        lock(&self.runs).insert(run_id, Arc::clone(&entry));

        entry
    }

    pub fn get(&self, run_id: Uuid) -> Option<Arc<RunEntry>> {
        self.evict_at(Instant::now());

        lock(&self.runs).get(&run_id).cloned()
    }

    pub fn len(&self) -> usize {
        lock(&self.runs).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // Takes the clock as a parameter so the limits are testable without waiting
    pub fn evict_at(&self, now: Instant) {
        let mut runs = lock(&self.runs);

        let mut finished: Vec<(Uuid, Instant)> = runs
            .iter()
            .filter_map(|(run_id, entry)| entry.terminal_at().map(|at| (*run_id, at)))
            .collect();

        for (run_id, finished_at) in &finished {
            if now.saturating_duration_since(*finished_at) >= self.retention {
                runs.remove(run_id);
            }
        }

        finished.retain(|(run_id, _)| runs.contains_key(run_id));
        finished.sort_by_key(|(_, finished_at)| *finished_at);

        let excess = finished.len().saturating_sub(self.max_terminal);

        for (run_id, _) in finished.into_iter().take(excess) {
            runs.remove(&run_id);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// Compares without stopping at the first difference. Only the length can show.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut difference = 0u8;

    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }

    difference == 0
}
