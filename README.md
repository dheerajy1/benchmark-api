# benchmark-api

A Rust HTTP API that benchmarks a local project. You give it a project directory, it builds and starts the project, drives HTTP load against it, measures it, stores the result in PostgreSQL and lets you read the history back.

## Features

- Artifact size of source, build output and excluded files
- Build time
- Startup time until the application answers its readiness check
- Memory (RSS) baseline and peak under load
- CPU baseline, average and peak under load
- HTTP latency percentiles and throughput, by request count or sustained duration
- Persistent history in PostgreSQL with filters and cursor pagination
- Comparison of two stored runs with backend-decided improvement direction

Memory and CPU measurement is Linux only (`/proc`).

## Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/api/v1/benchmarks` | Run a benchmark and store it |
| `GET` | `/api/v1/benchmarks` | List historical benchmark summaries |
| `GET` | `/api/v1/benchmarks/{run_id}` | Get one stored benchmark in the POST result shape |
| `GET` | `/api/v1/benchmarks/overview` | Dashboard totals, the latest run and the trend of the newest 30 runs |
| `GET` | `/api/v1/comparisons/summary?runIds=<baseline>,<comparison>` | Compare two stored runs |

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

## Documentation

| Document | Content |
| --- | --- |
| [docs/architecture.md](docs/architecture.md) | Layers, modules, request flow, error and response shapes |
| [docs/lifecycle.md](docs/lifecycle.md) | The benchmark lifecycle step by step |
| [docs/api.md](docs/api.md) | Endpoints, parameters, pagination, errors |
| [docs/development.md](docs/development.md) | Commands, tests, conventions |
| [docs/postgres/README.md](docs/postgres/README.md) | Database setup, migrations, schema, maintenance |

## License

See [LICENSE](LICENSE).
