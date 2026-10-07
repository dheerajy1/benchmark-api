use std::collections::HashMap;

use super::proc::ProcessTicks;

// Accumulates CPU ticks per process so exited processes keep their contribution
#[derive(Debug, Default)]
pub struct CpuTracker {
    last_seen: HashMap<u32, u64>,
    consumed: u64,
}

impl CpuTracker {
    // Ticks already present in the reference reading are not counted
    pub fn from_reference(reference: &[ProcessTicks]) -> Self {
        let last_seen = reference
            .iter()
            .map(|item| (item.pid, item.ticks))
            .collect();

        Self {
            last_seen,
            consumed: 0,
        }
    }

    // Returns ticks consumed since the previous observation
    // A process not in the reference counts all of its ticks
    // A process missing from this reading keeps its last seen ticks
    pub fn observe(&mut self, reading: &[ProcessTicks]) -> u64 {
        let mut delta = 0u64;

        for item in reading {
            let previous = self.last_seen.get(&item.pid).copied().unwrap_or(0);

            delta = delta.saturating_add(item.ticks.saturating_sub(previous));

            self.last_seen.insert(item.pid, item.ticks);
        }

        self.consumed = self.consumed.saturating_add(delta);

        delta
    }

    // Total ticks consumed since the reference reading
    pub fn consumed_ticks(&self) -> u64 {
        self.consumed
    }
}
