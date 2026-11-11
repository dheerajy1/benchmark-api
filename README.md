# benchmark-api

A Rust HTTP API that benchmarks a local project. You give it a project directory, it builds and starts the project, drives HTTP load against it, measures it, stores the result in PostgreSQL and lets you read the history back.

## Features

- Artifact size of source, build output and excluded files
- Build time
- Startup time until the application answers its readiness check
- Memory (RSS) baseline and peak under load
- CPU baseline, average and peak under load
- HTTP latency percentiles and throughput, by request count or sustained duration
- Live runs that can be watched over a WebSocket and cancelled
- Persistent history in PostgreSQL with status, runtime, path and text filters, counts and cursor pagination. Completed, failed and cancelled runs are all stored, and a run can be deleted (soft delete)
- A dashboard per project with totals, the latest two runs and a trend
- Stage averages of a project for the estimate of a new run
- Comparison of two stored runs with backend-decided improvement direction

Memory and CPU measurement is Linux only (`/proc`).

## Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/api/v1/live-runs` | Start a benchmark, it is stored when it ends |
| `GET` | `/api/v1/live-runs/active` | The run that executes now, or `null` |
| `GET` | `/api/v1/live-runs/{run_id}` | Replay the events of a run |
| `GET` | `/api/v1/live-runs/{run_id}/ws` | Follow a run over a WebSocket |
| `POST` | `/api/v1/live-runs/{run_id}/cancel` | Cancel a run |
| `GET` | `/api/v1/benchmarks` | List historical benchmark summaries with counts |
| `GET` | `/api/v1/benchmarks/{run_id}` | Get one stored benchmark with its status and stage times |
| `DELETE` | `/api/v1/benchmarks/{run_id}` | Delete a run (it is hidden from every read) |
| `GET` | `/api/v1/benchmarks/overview?target_path=` | Dashboard of one project: totals, latest two runs, trend |
| `GET` | `/api/v1/benchmarks/stage-averages?target_path=` | Average stage durations of the last 3 done runs |
| `GET` | `/api/v1/comparisons/summary?runIds=<baseline>,<comparison>` | Compare two stored runs |
| `GET` | `/api/v1/health` | API and database status |

Request and response details are in [docs/api.md](docs/api.md).

## Quick start

Requirements: a Rust toolchain (edition 2024) and a local PostgreSQL.

1. Create the database and role, see [docs/postgres/README.md](docs/postgres/README.md).
2. Copy the environment template and fill it in:

```bash
cp .env.example .env
```

3. Start the API. Pending migrations are applied at startup:

```bash
cargo run
```

## Configuration

| Variable | Purpose |
| --- | --- |
| `APP_PORT` | Port the API listens on, bound to `127.0.0.1` |
| `APP_HEALTH_CLIENT_ID` | Default readiness check credential |
| `APP_HEALTH_CLIENT_SECRET` | Default readiness check credential |
| `DATABASE_URL` | PostgreSQL connection string |
| `TEST_DATABASE_URL` | Optional, enables the real-database tests, passed on the command line only |

## Browser access (CORS)

The API answers browsers from these origins only: `http://localhost:5002`, `http://127.0.0.1:5002`, `http://localhost:8092` and `http://127.0.0.1:8092` (the API's own ports), `http://localhost:5003`, `http://127.0.0.1:5003`, `http://localhost:8093` and `http://127.0.0.1:8093` (the dashboard, 5003 in development and 8093 in production), and the `*.usw3.devtunnels.ms` and `*.trycloudflare.com` tunnels. The list is in `src/origin.rs` and the WebSocket handshake uses the same one. Allowed methods are `GET`, `POST` and `DELETE`, and the `x-request-id` response header is exposed so the dashboard can show it. Details are in [docs/api.md](docs/api.md#cors).

## Documentation

| Document | Content |
| --- | --- |
| [docs/architecture.md](docs/architecture.md) | Layers, modules, the benchmark lifecycle, request flow, error and response shapes |
| [docs/api.md](docs/api.md) | Endpoints, saved targets, parameters, pagination, errors, request ids, CORS |
| [docs/live-runs.md](docs/live-runs.md) | Live runs, events, WebSocket, samples, cancelling |
| [docs/development.md](docs/development.md) | Commands, tests, conventions |
| [docs/postgres/README.md](docs/postgres/README.md) | Database setup, migrations, schema, maintenance |

## License

See [LICENSE](LICENSE).
