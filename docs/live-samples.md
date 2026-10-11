# Live samples (Phase 2)

A run that somebody watches emits one `sample` event every 500 ms while the workload runs.
A run nobody watches (the legacy endpoint, the existing tests) measures and emits nothing
extra: `ProgressSink::wants_samples()` is false for the no-op sink, so no gauge, counter or
tick exists for it.

## Event

```json
{"type":"sample","data":{
  "workload_elapsed_ms": 2000,
  "memory": { "rss_bytes": 80123904, "peak_rss_bytes": 81920000 },
  "cpu": { "interval_percent": 41.23, "average_percent": 38.9, "peak_percent": 55.0 },
  "http": {
    "completed": 412, "successful": 412, "failed": 0, "requests_per_second": 206.0,
    "progress": { "mode": "requests", "done": 412, "total": 1000 }
  }
}}
```

- `progress` in duration mode: `{"mode":"duration","elapsed_ms":2000,"total_ms":30000}`.
- `memory` and `cpu` are `null` until a reading exists. Inside `cpu`, `interval_percent`
  is `null` when the interval was shorter than 50 ms and `peak_percent` is `null` until
  the first valid interval. `average_percent` is `null` until the first reading.
- `failed` is `completed - successful`. A snapshot never shows more successful than
  completed requests.
- Values are rounded to two decimals, like the final result.
- No latency statistic is ever part of a sample: p50, p95, p99, min, mean and max are
  computed from every request at the end and only appear in `run.completed`.

## Where each value comes from

| Value | Source |
|---|---|
| `memory.rss_bytes`, `peak_rss_bytes` | the existing 100 ms memory sampler, published on every reading |
| `cpu.interval_percent`, `peak_percent`, `average_percent` | the existing 100 ms CPU sampler, same formulas as the final result |
| `http.completed`, `successful` | two atomic counters the workload workers add to after every request |
| `http.requests_per_second` | `completed` over the workload time elapsed so far |
| `progress` | request mode: `completed` against the requested total; duration mode: elapsed time against the planned duration |

The samplers never emit events. The monitor loop of the workload executor, which already
wakes every 25 ms to watch the target, builds and emits the sample, so no thread is added.
The first sample comes at about 500 ms, so a workload shorter than that has none.

`baseline.measured` and the stage events are unchanged from Phase 1.

## Differences from the final result

The last live `requests_per_second` divides by the time of the last sample, the final one
by the full workload time including requests still in flight at the deadline, so the two
can differ slightly. The final CPU and memory peaks include readings taken after the last
sample.
