# PostgreSQL persistence

Every run that ends, completed, failed or cancelled, is stored in PostgreSQL in a single transaction by the live-run supervisor. If storing fails, nothing is stored and the terminal event of the run says `persisted: false`.

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
| `20261102000001_run_status_and_stages.sql` | `status`, `stopped_stage`, `failure_message` and `deleted_at` on `benchmark_runs`, the `benchmark_run_stages` table and the partial indexes of the visible runs |

The third migration adds `NOT NULL` columns without a default, so it expects a database without rows. The database was dropped and created again before it, and no old row is carried over.

Optional manual run with `sqlx-cli`:

```
sqlx migrate run --database-url "$DATABASE_URL"
```

Applied migrations are never edited. A schema change is a new migration file with a later version.

## Schema

```
runtimes, artifact_types, benchmark_statuses     lookup tables (id, code, name)
benchmark_runs                                   root, runtime_id is NULL when no runtime was detected
                                                 status (done, failed, cancelled), stopped_stage, failure_message, deleted_at
  benchmark_run_stages                           one row per stage that ran, in run order (position)
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
- `status` is checked by a `CHECK`, not an enum: `done`, `failed` or `cancelled`. A done run has no `stopped_stage` and no `failure_message`, only a failed run has a message and every failed run has one.
- `benchmark_run_stages.stage` is one of `discovery`, `artifact_size`, `build`, `startup`, `baseline`, `workload`, `workspaces`, `saving`, or `workspaces:<name>:native` and `workspaces:<name>:docker`. `duration_ms` is `NULL` when the stage has no duration. The `saving` stage is added by the repository at the end of the insert, so it measures the writes.
- A failed or cancelled run keeps the parts that finished. The parts it never reached have no row, so they read as `null`.
- Deleting a run is soft: `deleted_at` is set and the first deletion time is kept. Every read filters `deleted_at IS NULL`, and no row of any table is removed, so a restore is one update.

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

## Reading history

`GET /api/v1/benchmarks` and `GET /api/v1/benchmarks/{run_id}` read the same normalized tables. No history or summary table exists and none is needed.

- The list orders by `created_at DESC, id DESC` and pages with `(created_at, id) < (cursor)`. Filters on `runtime_id` (resolved from the runtime code), `target_path`, `status` and the search text are applied before the cursor. The search matches the path with `ILIKE` (the wildcards of the text are escaped) and the run id by its last characters or any part.
- The detail reads one run across every table, plus its stage rows, and rebuilds the result. Formatted strings such as `"50.00 MiB"` are computed again from the stored byte counts.
- The partial indexes only hold visible runs (`WHERE deleted_at IS NULL`): `(created_at DESC, id DESC)`, `(runtime_id, created_at DESC)`, `(target_path, created_at DESC)` and `(target_path, status, created_at DESC)`, which serves the done-only queries of one project (overview and stage averages).
- `benchmark_run_stages` has an index on `stage`, and a unique constraint on `(benchmark_run_id, stage)` and on `(benchmark_run_id, position)`, so a run lists its stages in order.

## Maintenance

Remove the privileges of the service role:

```sql
REVOKE ALL ON SCHEMA public FROM benchmark_api_service;
REVOKE ALL PRIVILEGES ON DATABASE benchmark_api_db FROM benchmark_api_service;
```

Drop a leftover test schema from a panicked test run:

```sql
DROP SCHEMA test_<uuid> CASCADE;
```

