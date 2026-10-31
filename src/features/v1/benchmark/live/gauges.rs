use std::{
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};

use crate::features::v1::benchmark::{
    metrics::http_latency::round_two_decimals, workload::plan::WorkloadLimit,
};

// How often a live sample is emitted while the workload runs
pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

// Requests finished so far, written by the workload workers and read by the sampler.
//
// Every request adds to `completed` first and to `successful` second, and a snapshot
// reads them in the opposite order, so a snapshot never shows more successful requests
// than completed ones.
#[derive(Default)]
pub struct RequestCounters {
    completed: AtomicU64,
    successful: AtomicU64,
}

impl RequestCounters {
    pub fn record(&self, success: bool) {
        self.completed.fetch_add(1, Ordering::SeqCst);

        if success {
            self.successful.fetch_add(1, Ordering::SeqCst);
        }
    }

    // (completed, successful)
    pub fn snapshot(&self) -> (u64, u64) {
        let successful = self.successful.load(Ordering::SeqCst);
        let completed = self.completed.load(Ordering::SeqCst);

        (completed, successful)
    }
}

// The latest RSS reading of the target and the running peak
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryGauge {
    pub rss_bytes: u64,
    pub peak_rss_bytes: u64,
}

// CPU of the target process group, every value is None until it can be measured
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuGauge {
    // CPU over the last sampling interval, None when the interval was too short
    pub interval_percent: Option<f64>,
    // CPU time over the wall-clock time since the workload started
    pub average_percent: Option<f64>,
    // Highest interval value so far
    pub peak_percent: Option<f64>,
}

// The latest values of a running workload. The memory and CPU samplers write them every
// 100 ms from their own threads, the workers count requests, and the monitor loop reads
// everything every SAMPLE_INTERVAL. It only holds values that are really measured.
#[derive(Default)]
pub struct LiveGauges {
    pub http: RequestCounters,
    memory: Mutex<Option<MemoryGauge>>,
    cpu: Mutex<Option<CpuGauge>>,
}

impl LiveGauges {
    pub fn set_memory(&self, rss_bytes: u64, peak_rss_bytes: u64) {
        *lock(&self.memory) = Some(MemoryGauge {
            rss_bytes,
            peak_rss_bytes,
        });
    }

    pub fn set_cpu(&self, gauge: CpuGauge) {
        *lock(&self.cpu) = Some(gauge);
    }

    pub fn memory(&self) -> Option<MemoryGauge> {
        *lock(&self.memory)
    }

    pub fn cpu(&self) -> Option<CpuGauge> {
        *lock(&self.cpu)
    }
}

// The data of one `sample` event. Latency percentiles are deliberately absent: they are
// only known when the workload has finished.
pub fn build_sample(gauges: &LiveGauges, limit: WorkloadLimit, elapsed: Duration) -> Value {
    let (completed, successful) = gauges.http.snapshot();
    let failed = completed.saturating_sub(successful);

    let requests_per_second = if elapsed.is_zero() {
        0.0
    } else {
        round_two_decimals(completed as f64 / elapsed.as_secs_f64())
    };

    let progress = match limit {
        WorkloadLimit::Requests(total) => json!({
            "mode": "requests",
            "done": completed,
            "total": total,
        }),
        WorkloadLimit::Duration(total) => json!({
            "mode": "duration",
            "elapsed_ms": elapsed.as_millis() as u64,
            "total_ms": total.as_millis() as u64,
        }),
    };

    let memory = gauges.memory().map(|memory| {
        json!({
            "rss_bytes": memory.rss_bytes,
            "peak_rss_bytes": memory.peak_rss_bytes,
        })
    });

    let cpu = gauges.cpu().map(|cpu| {
        json!({
            "interval_percent": cpu.interval_percent.map(round_two_decimals),
            "average_percent": cpu.average_percent.map(round_two_decimals),
            "peak_percent": cpu.peak_percent.map(round_two_decimals),
        })
    });

    json!({
        "workload_elapsed_ms": elapsed.as_millis() as u64,
        "memory": memory,
        "cpu": cpu,
        "http": {
            "completed": completed,
            "successful": successful,
            "failed": failed,
            "requests_per_second": requests_per_second,
            "progress": progress,
        },
    })
}

// The data of one `workspace.sample` event: the latest memory and CPU of the measured
// workspace mode, the same shapes the `sample` event uses
pub fn build_workspace_sample(
    gauges: &LiveGauges,
    workspace: &str,
    mode: &str,
    elapsed: Duration,
) -> Value {
    let memory = gauges.memory().map(|memory| {
        json!({
            "rss_bytes": memory.rss_bytes,
            "peak_rss_bytes": memory.peak_rss_bytes,
        })
    });

    let cpu = gauges.cpu().map(|cpu| {
        json!({
            "interval_percent": cpu.interval_percent.map(round_two_decimals),
            "average_percent": cpu.average_percent.map(round_two_decimals),
            "peak_percent": cpu.peak_percent.map(round_two_decimals),
        })
    });

    json!({
        "workspace": workspace,
        "mode": mode,
        "elapsed_ms": elapsed.as_millis() as u64,
        "memory": memory,
        "cpu": cpu,
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
