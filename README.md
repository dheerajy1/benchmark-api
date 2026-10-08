# benchmark-api

benchmark api written in rust

cargo clean

cargo fmt

cargo fmt --check

cargo check

cargo test

cargo fmt --check && cargo check

cargo fmt --check && cargo check && cargo test

cargo fmt --check && cargo fmt && cargo check && cargo test

cargo fmt && cargo check

cargo fmt && cargo check && cargo test

cargo clean && cargo fmt && cargo check

cargo run

---

# Lifecycle
```
prepare target
    ↓
discover
    ↓
build
    ↓
start application
    ↓
wait until ready
    ↓
┌─────────────────────────────┐
│ application stays alive     │
│                             │
│ memory baseline             │
│ HTTP latency                │
│ throughput                  │
│ CPU                         │
│ memory under load           │
└─────────────────────────────┘
    ↓
graceful shutdown
    ↓
assemble BenchmarkResult
```

# Persistence

Every successful benchmark is stored in PostgreSQL in one transaction. See `docs/postgres/README.md` for setup, migrations and tests.

```
service::run -> BenchmarkResult -> BenchmarkRepository::save (one transaction) -> HTTP response
```

# Architecture

```
prepare
  ↓
discover
  ↓
build
  ↓
TargetProcess.start()
  ↓
wait_until_ready()
  ↓
startup_time
  ↓
memory baseline + 5s peak sampling
  ↓
[future runtime/load metrics]
  ↓
TargetProcess.shutdown()
```
---

TEST_DATABASE_URL=postgres://benchmark_api_service:<password>@localhost:5432/benchmark_api_db cargo test --test persistence

---

REVOKE ALL ON SCHEMA public FROM benchmark_api_service;
REVOKE ALL PRIVILEGES ON DATABASE benchmark_api_db FROM benchmark_api_service;

---

DROP SCHEMA test_01a11a058a1c722e8c1927626bb03475 CASCADE;
DROP SCHEMA test_01a11a0587b477478286dcd1aaf3f788 CASCADE;

---
**Step 1: Show every stored table for run `01a11a1c-f037-731f-9005-d3045484d347` in one row**

```sql
SELECT
    r.id,
    r.target_path,
    rt.code AS runtime,
    r.created_at,
    bs.code AS build_status,
    b.duration_ms AS build_ms,
    b.args AS build_args,
    b.started_at AS build_started_at,
    b.exit_code AS build_exit_code,
    ss.code AS startup_status,
    s.duration_ms AS startup_ms,
    s.ready_at,
    s.readiness_port,
    m.baseline_rss_bytes,
    m.peak_rss_bytes,
    m.delta_rss_bytes,
    c.average_percent,
    c.peak_percent,
    hs.code AS http_status,
    h.total_requests,
    h.requests_per_second,
    h.p50_ms,
    h.p99_ms
FROM benchmark_runs r
LEFT JOIN runtimes rt ON rt.id = r.runtime_id
LEFT JOIN benchmark_builds b ON b.benchmark_run_id = r.id
LEFT JOIN benchmark_statuses bs ON bs.id = b.status_id
LEFT JOIN benchmark_startups s ON s.benchmark_run_id = r.id
LEFT JOIN benchmark_statuses ss ON ss.id = s.status_id
LEFT JOIN benchmark_memories m ON m.benchmark_run_id = r.id
LEFT JOIN benchmark_cpus c ON c.benchmark_run_id = r.id
LEFT JOIN benchmark_http_latencies h ON h.benchmark_run_id = r.id
LEFT JOIN benchmark_statuses hs ON hs.id = h.status_id
WHERE r.id = '01a11a1c-f037-731f-9005-d3045484d347';
```