# Benchmark lifecycle

## Steps

A benchmark owns the target process from start to shutdown. Every measurement happens while the same process stays alive.

```mermaid
flowchart TD
    A[Validate request] --> B[Prepare target path]
    B --> C[Discover runtime]
    C --> D[Measure artifact size]
    D --> E[Build and measure build time]
    E --> F{Start command<br/>and readiness given?}
    F -- no --> N[startup_time status no_start_command]
    F -- yes --> G[Start target process]
    G --> H[Wait until ready]
    H --> I{Ready?}
    I -- no --> J[startup_time status failed, timeout or unhealthy]
    I -- yes --> K[Record startup_time]
    K --> L{Workload given?}
    L -- no --> M[Graceful shutdown]
    L -- yes --> W
    subgraph W [Application stays alive]
        direction TB
        W1[Stabilize 500 ms] --> W2[Memory and CPU baseline]
        W2 --> W3[HTTP workload]
        W3 --> W4[Sample RSS and CPU every 100 ms]
    end
    W --> M
    J --> M
    N --> R
    M --> R[Assemble BenchmarkResult]
    R --> S[(Store in PostgreSQL, one transaction)]
```

## Rules

- Nothing is inferred. Start command, readiness check and workload are only used when the request provides them.
- `memory`, `cpu` and `http_latency` are `null` when no workload ran or startup did not succeed. Nothing is fabricated.
- If the target dies during the workload, partial measurements are kept and `http_latency.status` is `target_exited`.
- A workload is either a fixed `requests` count or a sustained `duration_seconds`, never both.
- The target is always shut down, including after failures.

## Result

```text
data
├── run_id
├── target
├── discovery
└── metrics
    ├── artifact_size
    ├── build_time
    ├── startup_time
    ├── memory
    ├── cpu
    └── http_latency
```

`run_id` is a UUIDv7. Its embedded timestamp is the stored `created_at` of the run.

## Stored statuses

`success`, `failed`, `timeout`, `unhealthy`, `no_build_command`, `no_start_command`, `no_port`, `completed`, `target_exited`.
