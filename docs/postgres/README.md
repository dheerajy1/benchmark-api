# PostgreSQL persistence

Every successful `POST /api/v1/benchmarks` stores its `BenchmarkResult` in PostgreSQL in a single transaction. If storing fails, the endpoint returns `500 INTERNAL_SERVER_ERROR` and nothing is stored.

## Configuration

| Variable | Purpose |
| --- | --- |
| `DATABASE_URL` | PostgreSQL connection string, required at startup |
| `TEST_DATABASE_URL` | Optional, enables the real-database tests |

```
DATABASE_URL=postgres://benchmark_api_service:<password>@localhost:5432/benchmark_api_db
```

`.env.example` shows the format. The real `.env` stays untracked. `TEST_DATABASE_URL` is not read from `.env`, pass it on the command line (see Tests).

## First-time database setup

1. Replace `<CHANGE_ME>` with a real password in `docs/postgres/bootstrap.sql`. Do not commit it.
2. Run it once as a PostgreSQL administrator:

```
   psql -U postgres -f docs/postgres/bootstrap.sql
```

3. Put the matching `DATABASE_URL` in `.env`.

The bootstrap file creates the database `benchmark_api_db` and the login role `benchmark_api_service`, and grants the role what it needs:

- `ALL PRIVILEGES ON DATABASE benchmark_api_db` (connect, create schemas, temporary tables)
- `ALL ON SCHEMA public`, granted inside `benchmark_api_db` (required on PostgreSQL 15+ to create tables)

The role does not own the database. Using a GUI client instead of `psql`: remove the `\connect` line and run the last `GRANT` while connected to `benchmark_api_db`, otherwise the grant lands on the wrong database and the migrations fail with `permission denied for schema public`.

Tables, constraints, indexes and lookup rows are never created by hand.

## Migrations

Migrations live in `migrations/` and are embedded into the binary. The application applies pending migrations once at startup, before it accepts requests. Already applied migrations are skipped.

| Migration | Content |
| --- | --- |
| `20261007000001_create_schema.sql` | tables, foreign keys, unique and check constraints, indexes |
| `20261007000002_seed_lookup_data.sql` | `runtimes`, `artifact_types`, `benchmark_statuses` rows |

Optional manual run with `sqlx-cli`:

```
sqlx migrate run --database-url "$DATABASE_URL"
```

Applied migrations are never edited. A schema change is a new migration file with a later version.

## Schema

```
runtimes, artifact_types, benchmark_statuses     lookup tables (id, code, name)
benchmark_runs                                   root, runtime_id is NULL when no runtime was detected
  artifact_sizes                                 3 rows per run (source, build, excluded)
  benchmark_builds                               1 row per run
  benchmark_startups                             1 row per run
  benchmark_memories                             0..1 row per run
  benchmark_cpus                                 0..1 row per run
  benchmark_http_latencies                       0..1 row per run
```

- Primary keys are UUIDv7. Lookup rows have fixed ids in the seed migration, and application code always resolves them by `code`.
- No PostgreSQL enums. Allowed values are lookup rows.
- Timestamps are `TIMESTAMPTZ`, parsed from the RFC 3339 strings of the result. `created_at` is the timestamp inside the UUIDv7 `run_id`.
- Sizes and counts are `BIGINT`. Formatted strings such as `"50.00 MiB"` are never stored.
- CPU percentages and HTTP latencies are `NUMERIC`.
- Memory, CPU and HTTP latency rows are absent when no workload ran.
- Skipped steps (`no_build_command`, `no_start_command`, `no_port`) still get a row, and their unmeasured columns are `NULL`. `CHECK` constraints keep those columns consistent: a build is either entirely unmeasured or fully measured.
- Foreign keys use `ON DELETE RESTRICT`.

## Adding a lookup value

A new runtime, artifact type or status is a new migration that inserts the row with a fresh UUIDv7. Until then, persisting a result that uses it fails with an unknown lookup error and rolls back.

## Tests

`cargo test` needs no database: API tests use an in-memory repository and mapping tests are pure.

The real-database tests (`tests/persistence/`) run only when `TEST_DATABASE_URL` is set. Otherwise they print `skipped` and pass.

```
TEST_DATABASE_URL=postgres://benchmark_api_service:<password>@localhost:5432/benchmark_api_db cargo test --test persistence
```

Each test creates its own schema, migrates it from zero and drops it afterwards, so tests never touch the real tables. A test that panics leaves its `test_...` schema behind, drop it with `DROP SCHEMA <name> CASCADE;`.

## Verify a stored run

```sql
SELECT r.id, r.target_path, rt.code AS runtime, r.created_at,
       bs.code AS build_status, b.duration_ms AS build_ms,
       ss.code AS startup_status, s.duration_ms AS startup_ms,
       m.peak_rss_bytes, c.average_percent,
       hs.code AS http_status, h.total_requests, h.requests_per_second
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
ORDER BY r.created_at DESC
LIMIT 1;
```