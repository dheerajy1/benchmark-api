use std::{
    collections::VecDeque,
    time::{Instant, SystemTime},
};

use serde_json::Value;
use uuid::Uuid;

use super::event::{self, CONTRACT_VERSION, RunEvent, kind};

// Samples kept per run, 10 minutes at the 500 ms live interval
pub const SAMPLE_RING_CAPACITY: usize = 1_200;

// Hard bound on the structural events of one run, the real count is about 30.
// One slot is always kept free so a terminal event can never be refused.
pub const MAX_STRUCTURAL_EVENTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendError {
    // The run already has a terminal event, nothing may follow it
    AlreadyTerminal,
    // A non-terminal structural event beyond the hard bound
    StructuralLimit,
}

// What a client receives when it asks for the events after a sequence number
#[derive(Debug, Clone)]
pub struct Replay {
    // Retained events with seq greater than `after`, in seq order
    pub events: Vec<RunEvent>,
    pub status: RunStatus,
    pub last_seq: u64,
    // The seq of the newest sample that was evicted, 0 while none was
    pub dropped_through_seq: u64,
}

impl Replay {
    pub fn truncated(&self) -> bool {
        self.dropped_through_seq > 0
    }
}

// The ordered, bounded event store of one run.
//
// Structural events (run, stage, measurement, terminal) are never evicted.
// Samples live in a ring, the oldest is evicted first. One counter numbers both,
// so evicting samples leaves holes in the sequence, and only there: a hole is
// legitimate exactly while `seq <= dropped_through_seq`.
pub struct EventLog {
    run_id: Uuid,
    started: Instant,
    sample_capacity: usize,
    structural: Vec<RunEvent>,
    samples: VecDeque<RunEvent>,
    last_seq: u64,
    dropped_through_seq: u64,
    status: RunStatus,
    current_stage: Option<String>,
    terminal_at: Option<Instant>,
}

impl EventLog {
    pub fn new(run_id: Uuid) -> Self {
        Self::with_sample_capacity(run_id, SAMPLE_RING_CAPACITY)
    }

    pub fn with_sample_capacity(run_id: Uuid, sample_capacity: usize) -> Self {
        Self {
            run_id,
            started: Instant::now(),
            sample_capacity,
            structural: Vec::new(),
            samples: VecDeque::new(),
            last_seq: 0,
            dropped_through_seq: 0,
            status: RunStatus::Running,
            current_stage: None,
            terminal_at: None,
        }
    }

    // Numbers the event, stores it and returns its seq. A terminal event also moves
    // the status, in the same step, so nothing can be appended after it.
    pub fn append(&mut self, event_kind: &'static str, data: Value) -> Result<u64, AppendError> {
        if self.status != RunStatus::Running {
            return Err(AppendError::AlreadyTerminal);
        }

        let terminal = event::is_terminal(event_kind);
        let sample = event::is_sample(event_kind);

        if !terminal && !sample && self.structural.len() >= MAX_STRUCTURAL_EVENTS - 1 {
            return Err(AppendError::StructuralLimit);
        }

        let seq = self.last_seq + 1;

        if event_kind == kind::STAGE_STARTED {
            if let Some(stage) = data.get("stage").and_then(Value::as_str) {
                self.current_stage = Some(stage.to_string());
            }
        }

        let run_event = RunEvent {
            v: CONTRACT_VERSION,
            run_id: self.run_id,
            seq,
            t_ms: self.started.elapsed().as_millis() as u64,
            at: event::format_timestamp(SystemTime::now()),
            kind: event_kind,
            data,
        };

        if sample {
            self.samples.push_back(run_event);

            if self.samples.len() > self.sample_capacity {
                if let Some(evicted) = self.samples.pop_front() {
                    self.dropped_through_seq = evicted.seq;
                }
            }
        } else {
            self.structural.push(run_event);
        }

        self.last_seq = seq;

        if terminal {
            self.status = if event_kind == kind::RUN_COMPLETED {
                RunStatus::Completed
            } else {
                RunStatus::Failed
            };
            self.terminal_at = Some(Instant::now());
        }

        Ok(seq)
    }

    pub fn replay(&self, after: u64) -> Replay {
        let mut events: Vec<RunEvent> = self
            .structural
            .iter()
            .chain(self.samples.iter())
            .filter(|run_event| run_event.seq > after)
            .cloned()
            .collect();

        events.sort_by_key(|run_event| run_event.seq);

        Replay {
            events,
            status: self.status,
            last_seq: self.last_seq,
            dropped_through_seq: self.dropped_through_seq,
        }
    }

    pub fn status(&self) -> RunStatus {
        self.status
    }

    pub fn is_terminal(&self) -> bool {
        self.status != RunStatus::Running
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    // The stage that was started last, used to say where a run failed
    pub fn current_stage(&self) -> Option<&str> {
        self.current_stage.as_deref()
    }

    pub fn terminal_at(&self) -> Option<Instant> {
        self.terminal_at
    }
}
