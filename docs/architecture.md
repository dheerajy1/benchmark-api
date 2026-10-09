# Architecture

## Overview

The API has three layers. HTTP handlers know nothing about SQL, the benchmark domain knows nothing about the database driver, and PostgreSQL is only reached through one repository trait.

```mermaid
flowchart TD
    Client([Client]) --> Router[axum router]
    Router --> Handler[HTTP handlers]
    Handler --> Run[benchmark service<br/>runs a benchmark]
    Handler --> History[history service<br/>reads past benchmarks]
    Run --> Repo
    History --> Repo[BenchmarkRepository trait]
    Repo --> Pg[PgBenchmarkRepository]
    Pg --> DB[(PostgreSQL)]
    Run --> Target[[Target project<br/>child process]]
```

## Modules

```text
src/
├── main.rs                 startup: env, database, migrations, server
├── app.rs                  router assembly (shared by main and API tests)
├── app_state.rs            Env + Arc<dyn BenchmarkRepository>
├── config/                 Env and DatabaseConfig loading
├── schemas/                validated environment shapes
├── errors/                 global response shapes and error catalogue
├── features/v1/benchmark/
│   ├── handler.rs          POST route, route table
│   ├── service.rs          one benchmark run, start to result
│   ├── discovery.rs        runtime detection
│   ├── metrics/            artifact size, build, startup, memory, CPU, HTTP latency
│   ├── workload/           HTTP load plan, executor, observations
│   ├── target_process.rs   start and stop the target process group
│   ├── history/            GET list and GET detail (handler, service, query, cursor, model)
│   └── model.rs            BenchmarkResult, the shape POST returns and GET detail rebuilds
├── features/v1/overview/
│   ├── handler.rs          GET /benchmarks/overview
│   ├── service.rs          count, latest through the history service, trend points
│   ├── convert.rs          trend row to API point, no calculation
│   ├── error.rs            storage failures
│   └── model.rs            the overview response
├── features/v1/comparison/
│   ├── handler.rs          GET /comparisons/summary
│   ├── query.rs            strict runIds parsing
│   ├── service.rs          loads the two runs, reports missing ones
│   ├── compare.rs          pure calculation, two runs in, one comparison out
│   ├── metric.rs           difference, percentage change, direction
│   └── model.rs            the comparison response
└── persistence/
    ├── repository.rs       BenchmarkRepository trait
    ├── history.rs          storage-side types of the history list
    ├── stored_run.rs       one rebuilt run plus its created_at
    ├── error.rs            PersistenceError
    └── postgres/           sqlx implementation
        ├── insert.rs       write one run in one transaction
        ├── mapping.rs      BenchmarkResult to rows
        ├── summary.rs      list SQL
        ├── overview.rs     run count and trend SQL, comparison rules in the query
        ├── detail.rs       detail SQL, one run or several in two queries
        └── reconstruct.rs  rows back to BenchmarkResult
```

## Persistence boundary

```text
BenchmarkRepository
├── save(result)                  one transaction, all tables or none
├── list_summaries(query)         filters, cursor and limit applied in SQL
├── get_by_run_id(run_id)         rebuilds the POST result, None when absent
├── get_runs(run_ids)             rebuilds exactly the requested runs, absent ids are missing
├── count_runs()                  the number of every stored run
└── list_trends(limit)            newest runs, comparison rules applied in SQL, newest first
```

- API tests use an in-memory fake of the trait, so they need no database.
- Repository tests run against real PostgreSQL when `TEST_DATABASE_URL` is set.
- The normalized tables are the only source of truth. There are no summary or history tables and no caching.

## Write path

```mermaid
sequenceDiagram
    participant C as Client
    participant H as POST handler
    participant S as benchmark service
    participant R as Repository
    participant D as PostgreSQL
    C->>H: POST /api/v1/benchmarks
    H->>S: run (spawn_blocking)
    S-->>H: BenchmarkResult
    H->>R: save(result)
    R->>D: BEGIN, insert every table, COMMIT
    H-->>C: 200 success + data
```

If storing fails the endpoint returns `500` and nothing is stored.

## Read path

```mermaid
sequenceDiagram
    participant C as Client
    participant H as history handler
    participant S as history service
    participant R as Repository
    participant D as PostgreSQL
    C->>H: GET /api/v1/benchmarks?limit&before&runtime&target_path
    H->>H: validate query, decode cursor
    H->>S: list(params)
    S->>R: list_summaries(limit + 1)
    R->>D: filters, (created_at, id) cursor, ORDER BY, LIMIT
    D-->>R: rows
    S-->>H: items + pagination
    H-->>C: 200 success + data
```

The service asks for one row more than `limit`. The extra row only answers "is there another page" and is never returned.

## Overview path

```mermaid
sequenceDiagram
    participant C as Client
    participant H as overview handler
    participant S as overview service
    participant R as Repository
    participant D as PostgreSQL
    C->>H: GET /api/v1/benchmarks/overview
    H->>S: overview()
    S->>R: count_runs()
    R->>D: COUNT(*) FROM benchmark_runs
    S->>R: list_summaries(limit 1) through the history service
    R->>D: newest run, ORDER BY created_at DESC, id DESC
    S->>R: list_trends(30)
    R->>D: newest 30 runs, status joins decide which metrics are null
    D-->>R: rows
    S-->>H: totals + latest + trends
    H-->>C: 200 success + data
```

The overview is derived and read-only. It adds no table, view, cache or job. The trend query joins `benchmark_statuses` for the build, the startup and the HTTP result, so a build or startup duration is only returned for a `success` status and the HTTP numbers only for a `completed` workload with at least one request. `latest` reuses the history summary and keeps its raw values, which is why it can differ from the first trend point of the same run.

## Comparison path

```mermaid
sequenceDiagram
    participant C as Client
    participant H as comparison handler
    participant S as comparison service
    participant R as Repository
    participant D as PostgreSQL
    C->>H: GET /api/v1/comparisons/summary?runIds=a,b
    H->>H: validate: exactly two different UUIDs
    H->>S: compare(baseline, comparison)
    S->>R: get_runs([a, b])
    R->>D: runs WHERE id = ANY(...), then their artifact sizes
    D-->>R: the two runs only
    S->>S: calculate differences, percentage change, direction
    S-->>H: comparison, or 404 naming the missing ids
    H-->>C: 200 success + data
```

A comparison is derived and read-only. It adds no table, stores nothing and loads no unrelated run.

## Response shapes

```json
{ "success": true, "httpCode": 200, "message": "...", "data": {} }
```

```json
{ "success": false, "httpCode": 400, "code": "VALIDATION_ERROR", "message": "...", "error": "..." }
```

| Code | HTTP | Used for |
| --- | --- | --- |
| `BAD_REQUEST` | 400 | malformed JSON, bad path, malformed `run_id` |
| `VALIDATION_ERROR` | 400 | invalid body fields, bad query parameters, bad cursor |
| `NOT_FOUND` | 404 | unknown route, unknown run |
| `METHOD_NOT_ALLOWED` | 405 | wrong method on an existing route |
| `INTERNAL_SERVER_ERROR` | 500 | benchmark failure after validation, storage failure |
