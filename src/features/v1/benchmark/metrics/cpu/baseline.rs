use std::{
    thread,
    time::{Duration, Instant},
};

use super::{
    calc::cpu_percent,
    proc::{ProcessTicks, read_group_ticks},
    tracker::CpuTracker,
};

// Baseline CPU of the process group over one window, measured right before the workload
pub fn measure_baseline_for_group(window: Duration, group_id: u32) -> Result<f64, String> {
    measure_baseline(window, move || read_group_ticks(group_id))
}

// Two readings one window apart, percent is the CPU time between them over wall-clock time
pub fn measure_baseline<F>(window: Duration, mut reader: F) -> Result<f64, String>
where
    F: FnMut() -> Result<Vec<ProcessTicks>, String>,
{
    let first = reader()?;

    // The clock starts after the first reading so both ends use the same convention
    let started = Instant::now();
    let mut tracker = CpuTracker::from_reference(&first);

    thread::sleep(window);

    let second = reader()?;
    let elapsed = started.elapsed();

    let ticks = tracker.observe(&second);

    Ok(cpu_percent(ticks, elapsed))
}
