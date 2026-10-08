# Development

## Commands

| Command | Purpose |
| --- | --- |
| `cargo fmt` | Format the code |
| `cargo fmt --check` | Check formatting without changing files |
| `cargo check` | Type check |
| `cargo test` | Run the tests |
| `cargo run` | Start the API |
| `cargo clean` | Remove build output |

Typical full check:

```bash
cargo fmt && cargo check && cargo test
```

## Tests

Tests live in `tests/`, never next to production code.

| Test crate | Content |
| --- | --- |
| `tests/benchmark.rs` | Metrics, workload, validation, and API tests with an in-memory repository |
| `tests/persistence.rs` | Real PostgreSQL tests for mapping, constraints, migrations, repository, history and comparison |

The PostgreSQL tests are skipped unless `TEST_DATABASE_URL` is set. Pass it on the command line, not in `.env`:

```bash
TEST_DATABASE_URL=postgres://benchmark_api_service:<password>@localhost:5432/benchmark_api_db cargo test --test persistence
```

Each test creates its own throwaway schema named `test_<uuid>` and drops it afterwards. A test that panics leaves its schema behind. Remove leftovers:

```sql
SELECT schema_name
FROM information_schema.schemata;
```

```sql
SELECT tablename
FROM pg_tables
WHERE schemaname = 'public';
```

```sql
SELECT tablename
FROM pg_tables
WHERE schemaname = 'test_01a11a0587b477478286dcd1aaf3f788';
```

```sql
DROP SCHEMA test_<uuid> CASCADE;
```

## Conventions

- Comments go on their own line above the code, never inline.
- HTTP handlers contain no SQL. Database access lives in `src/persistence`.
- Applied migrations are never edited, a schema change is a new migration.
- Error responses use the catalogue in `src/errors/catalog.rs`.

## Inspect one stored run

Replace the id and run it with `psql`:

```sql
SELECT
    r.id, r.target_path, rt.code AS runtime, r.created_at,
    bs.code AS build_status, b.duration_ms AS build_ms,
    ss.code AS startup_status, s.duration_ms AS startup_ms,
    m.baseline_rss_bytes, m.peak_rss_bytes, m.delta_rss_bytes,
    c.average_percent, c.peak_percent,
    hs.code AS http_status, h.total_requests, h.requests_per_second, h.p50_ms, h.p99_ms
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
WHERE r.id = '<run_id>';
```
