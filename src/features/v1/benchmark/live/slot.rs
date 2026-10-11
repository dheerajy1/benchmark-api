use std::sync::{Arc, Mutex, MutexGuard};

use uuid::Uuid;

// Which submission path owns the execution slot
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    // The blocking POST /api/v1/benchmarks
    Legacy,
    // A background live run
    Live,
}

// The run that currently holds the slot, as reported to a rejected submission
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRun {
    pub kind: RunKind,
    // Known for live runs, a legacy run creates its id later inside the execution
    pub run_id: Option<Uuid>,
}

// The one execution slot of the process, shared by every submission path.
// At most one benchmark executes at a time because concurrent runs distort the
// CPU and memory numbers and compete for the same ports.
#[derive(Default)]
pub struct RunSlot {
    active: Mutex<Option<ActiveRun>>,
}

impl RunSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    // Atomic: of any number of concurrent callers exactly one gets the permit
    pub fn try_acquire(
        self: &Arc<Self>,
        kind: RunKind,
        run_id: Option<Uuid>,
    ) -> Result<RunPermit, ActiveRun> {
        let mut active = lock(&self.active);

        if let Some(current) = active.as_ref() {
            return Err(current.clone());
        }

        *active = Some(ActiveRun { kind, run_id });

        Ok(RunPermit {
            slot: Arc::clone(self),
            kind,
            run_id,
        })
    }

    pub fn active(&self) -> Option<ActiveRun> {
        lock(&self.active).clone()
    }
}

// Proof that the holder owns the execution slot, released when it is dropped.
// It can only be created by RunSlot::try_acquire.
pub struct RunPermit {
    slot: Arc<RunSlot>,
    kind: RunKind,
    run_id: Option<Uuid>,
}

impl RunPermit {
    pub fn kind(&self) -> RunKind {
        self.kind
    }

    pub fn run_id(&self) -> Option<Uuid> {
        self.run_id
    }
}

impl Drop for RunPermit {
    fn drop(&mut self) {
        *lock(&self.slot.active) = None;
    }
}

// A poisoned lock must never stop a permit from being released
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
