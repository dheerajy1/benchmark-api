# API reference

Base path: `/api/v1`. Every response uses the shapes in [architecture.md](architecture.md#response-shapes).

Every response carries an `x-request-id` header, and every error body repeats it as `requestId`. See [Request ids](#request-ids). Browser access is described in [CORS](#cors).

## Starting a run

`POST /live-runs` is the only way to start a benchmark. The answer, the events and the cancel call are in [live-runs-api.md](live-runs-api.md). The old blocking `POST /benchmarks` is removed, the `/benchmarks` route accepts `GET` only and a `POST` there answers `405 METHOD_NOT_ALLOWED`.

The request body is the one described here. The stored result of the run is read back with `GET /benchmarks/{run_id}`, and the run id of the 202 answer is that stored run id.

```json
{
  "path": "/home/me/projects/my-api",
  "startup": {
    "command": { "program": "bun", "args": ["run", "start"] },
    "readiness": {
      "type": "http", "host": "127.0.0.1", "port": 5001,
      "path": "/health", "method": "GET", "expected_status": 200
    }
  },
  "workload": {
    "method": "GET", "path": "/items",
    "requests": 1000, "concurrency": 50
  }
}
```

Only `path` is required. `workload` takes `requests` or `duration_seconds`, not both.

A raw request is refused with `403 FORBIDDEN` unless the server was started with `ALLOW_CUSTOM_RUNS=true`. A body that names a saved target (`{ "target_id": "...", "workload": { ... } }`) is handled by the saved-target rules in [saved-targets.md](saved-targets.md). Raw requests that are valid JSON but malformed are answered exactly as before this feature, with the same status, code, message and error text.

### Workspaces (optional)

A request may also carry a `workspaces` list. Each entry measures one extra process after the main project has finished, for example the recordlog subscriber, natively and in Docker. A project without a subscriber leaves the list out, and its request and response are exactly what they were before.

```json
{
  "path": "/home/me/projects/recordlog",
  "workspaces": [
    {
      "name": "subscriber",
      "directory": "subscriber",
      "env": { "file_path": "subscriber/.env" },
      "ready_log_line": "subscriber ready",
      "ready_timeout_seconds": 60,
      "load": { "command": "bun run load" },
      "idle_seconds": 30,
      "native": {
        "enabled": true,
        "build_command": "cargo build --release",
        "command": "./target/release/subscriber"
      },
      "docker": {
        "enabled": true,
        "build_command": "docker compose build subscriber",
        "build_timeout_seconds": 1800,
        "image": "recordlog-subscriber:latest",
        "container_name": "recordlog-subscriber",
        "run_command": "docker run -d --env-file {ENV_FILE} --name recordlog-subscriber recordlog-subscriber:latest"
      }
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `name` | Label in results, also the key that matches runs when comparing |
| `directory` | Folder of the workspace, relative to `path`, never leaving it |
| `env.file_path` or `env.pasted` | At most one. `pasted` is the body of a `.env` file (comments, `export`, quotes, blank lines, Windows line endings and `=` inside values are handled) |
| `ready_log_line` | Text that must appear in the output before the process counts as ready. Required when native or Docker is enabled |
| `ready_timeout_seconds` | How long to wait for that line, default 60 |
| `load.command` | Optional command run while memory is sampled |
| `idle_seconds` | Sampling window when there is no load command, default 30 |
| `native.enabled`, `native.command` | Start command of the plain process |
| `native.build_command` | Optional, runs once before the start command, for example a compile step |
| `docker.enabled` | Switches the Docker mode on |
| `docker.build_command` | Builds the image, its time is recorded |
| `docker.build_timeout_seconds` | Default 1800, so a Rust build is not cut off |
| `docker.image` | Image name, read for its size after the build |
| `docker.container_name` | Used to detect a running copy, find the process and remove the container |
| `docker.run_command` | Starts the container. Must contain `{ENV_FILE}` when an env is given, and may not when none is |

Commands are split into words like a shell would, but no shell runs, so nothing is expanded. Variable values are never stored or returned, only their names (`env_variable_names`).

A workspace is refused with `409 CONFLICT` before anything starts when a copy of it is already running: a container with the configured name or image, or a process whose command line holds the native command and whose working directory is the workspace folder. The message says what to stop.

Each enabled mode adds one entry to `metrics.workspaces`:

```json
{
  "name": "subscriber",
  "mode": "docker",
  "status": "success",
  "message": null,
  "env_variable_names": ["DATABASE_URL"],
  "build": { "status": "success", "duration_ms": 412000, "exit_code": 0 },
  "image": { "bytes": 98304000, "size": "93.75 MB", "disk_usage_bytes": 276000000, "disk_usage": "263.21 MB" },
  "startup": { "duration_ms": 1850 },
  "memory": { "baseline": {}, "peak": {}, "delta": {} },
  "cpu": { "baseline_percent": 0.0, "average_percent": 1.2, "peak_percent": 3.4 },
  "load": { "status": "success", "exit_code": 0, "duration_ms": 30500 },
  "idle_seconds": null
}
```

`mode` is `native` or `docker`. `image.bytes` is the image size `docker image inspect` reports, `image.disk_usage_bytes` the unpacked size on disk as `docker image ls` lists it (`null` when that Docker does not report it). A mode that fails to build, start or become ready keeps its `status` (`failed` or `timeout`), a `message` and `null` numbers, and the other mode still runs. `workspaces` is left out of the response when the run had none.

## GET /targets

Lists the saved targets from the targets file, with names only, never a header value. See [saved-targets.md](saved-targets.md) for the response shape and the rules.

## GET /benchmarks

Returns historical **summaries**, newest first. Deleted runs never appear.

| Parameter | Rule |
| --- | --- |
| `limit` | integer 1 to 100, default 20 |
| `before` | opaque cursor from a previous response |
| `runtime` | runtime lookup code such as `bun`, `node`, `python`, `rust`, `go` |
| `target_path` | exact match on the stored target path, trailing slashes are ignored |
| `status` | `done`, `failed` or `cancelled` |
| `search` | text, not empty. Matches the target path in any letter case, and a run id by its last characters or any part of it (the short id shown in the UI is the last 8 characters) |

Filters are applied first, then the cursor, then the limit, all in SQL.

```json
{
  "success": true,
  "httpCode": 200,
  "message": "Benchmarks retrieved.",
  "data": {
    "items": [
      {
        "run_id": "0192f3a0-0000-7000-8000-000000000000",
        "status": "done",
        "display_name": "my-api",
        "target": { "path": "/home/me/projects/my-api" },
        "discovery": { "runtime": "bun" },
        "created_at": "2026-10-08T05:11:22.123Z",
        "summary": {
          "build_output_bytes": 2400000,
          "workspaces": [
            {
              "name": "subscriber", "mode": "docker", "status": "success",
              "image_bytes": 98304000, "disk_usage_bytes": 276000000, "build_duration_ms": 412000
            }
          ],
          "build_time_ms": 1842,
          "startup_time_ms": 1500,
          "memory_peak_bytes": 73400320,
          "cpu_average_percent": 12.4,
          "requests_per_second": 121.45,
          "p95_ms": 8.7
        }
      }
    ],
    "counts": {
      "total": 12,
      "by_runtime": { "bun": 7, "rust": 4, "unknown": 1 },
      "by_status": { "done": 9, "failed": 2, "cancelled": 1 }
    },
    "pagination": { "limit": 20, "has_more": true, "next_cursor": "MTc5..." }
  }
}
```

- `status` is how the run ended, see [Run status](#run-status). `display_name` is the saved target name, otherwise the last folder of the target path.
- Every `summary` field is `null` when the run did not produce it, a `null` is never a zero. A failed or cancelled run keeps the parts it finished and has `null` for the rest. `discovery.runtime` is `null` when no runtime was detected.
- `summary.workspaces` has one entry per workspace and mode, `[]` when the run had none.
- `next_cursor` is `null` on the last page.
- Treat the cursor as opaque, send it back unchanged as `before`.
- Runs are ordered by `created_at DESC, id DESC`, so runs with the same timestamp keep a stable order across pages.

### Counts

`counts` is for the filter pills and the footer. It follows every active filter except the facet it describes, so each pill shows what you get when you pick it.

| Field | Follows |
| --- | --- |
| `total` | every filter. Not changed by `limit` or `before` |
| `by_runtime` | every filter except `runtime`. A run without a runtime is counted under `unknown`, a runtime without a run is absent |
| `by_status` | every filter except `status`. `done`, `failed` and `cancelled` are always present, `0` when empty |

### Run status

| Status | Meaning |
| --- | --- |
| `failed` | the run failed on its own, or the build, the startup or the HTTP load ended with `failed`, `timeout`, `unhealthy` or `target_exited` |
| `cancelled` | the run was stopped on request |
| `done` | everything else |

A workspace that fails does not fail the run. A build or startup that was skipped (`no_build_command`, `no_start_command`, `no_port`) does not fail the run either.

### Errors

| Case | Response |
| --- | --- |
| `limit` is 0, negative, above 100 or not an integer | 400 `VALIDATION_ERROR` |
| `before` is not a cursor from this API | 400 `VALIDATION_ERROR` |
| `runtime` is empty or not a known runtime code | 400 `VALIDATION_ERROR` |
| `target_path` is empty | 400 `VALIDATION_ERROR` |
| `status` is not `done`, `failed` or `cancelled` | 400 `VALIDATION_ERROR` |
| `search` is empty | 400 `VALIDATION_ERROR` |
| storage failure | 500 `INTERNAL_SERVER_ERROR` |

## GET /benchmarks/{run_id}

Returns the benchmark as it was measured, the same fields as the result of a finished run, plus how the run ended.

| Added field | Meaning |
| --- | --- |
| `status` | `done`, `failed` or `cancelled` |
| `stopped_stage` | the stage a failed or cancelled run stopped in, `null` for a done run |
| `failure_message` | why a failed run failed, `null` for the others |
| `display_name` | the saved target name, otherwise the last folder of the target path |
| `stages` | `[{ "stage", "status", "duration_ms" }]` in run order, `[]` when the run has no stage rows |

A part the run never reached is `null`, never a zero. The stage keys are `discovery`, `artifact_size`, `build`, `startup`, `baseline`, `workload`, `workspaces` and `saving`, and `workspaces:<name>:native` or `workspaces:<name>:docker` for one mode of one workspace. A stage that was running when the run stopped keeps the status `cancelled` or `failed`. `duration_ms` is `null` when the stage has no duration.

| Case | Response |
| --- | --- |
| run exists | 200 with the benchmark |
| valid UUID, no such run, or the run was deleted | 404 `NOT_FOUND` |
| `run_id` is not a UUID | 400 `BAD_REQUEST` |
| a method other than `GET` or `DELETE` | 405 `METHOD_NOT_ALLOWED` |

`discovery.build` is always `null`, as it is in the result of a finished run.

## DELETE /benchmarks/{run_id}

Deletes a run. The run is only marked as deleted, its rows stay in the database, and every read stops seeing it: the list, the counts, the detail, the overview, the stage averages, Compare and the configuration endpoints. Deleting a run that is already deleted is not an error.

```json
{ "success": true, "httpCode": 200, "message": "Benchmark deleted.", "data": null }
```

| Case | Response |
| --- | --- |
| deleted, or already deleted | 200 with `data: null` |
| valid UUID that was never stored | 404 `NOT_FOUND` |
| `run_id` is not a UUID | 400 `BAD_REQUEST` |
| storage failure | 500 `INTERNAL_SERVER_ERROR` |

## GET /benchmarks/overview

Returns the dashboard of one project: how its runs ended, its newest two done runs and a trend over its newest done runs. A project is one stored target path. Every part belongs to the same project and everything is calculated from the stored runs on each request, nothing is cached. Deleted runs are never counted.

```text
GET /api/v1/benchmarks/overview?target_path=<path>
```

| Parameter | Rule |
| --- | --- |
| `target_path` | optional, not empty, trailing slashes are ignored. Without it, the project of the newest `done` run is shown, or the project of the newest run when no run is done |

Other parameters are ignored.

```json
{
  "success": true,
  "httpCode": 200,
  "message": "Benchmark overview retrieved.",
  "data": {
    "project": { "path": "/home/me/projects/my-api", "name": "my-api", "runtime": "bun", "source": "target" },
    "projects": [
      {
        "path": "/home/me/projects/my-api", "name": "my-api", "runtime": "bun",
        "total_runs": 9, "done_runs": 7, "last_run_at": "2026-10-08T05:11:22.123Z"
      }
    ],
    "totals": { "total_runs": 9, "done_runs": 7, "failed_runs": 1, "cancelled_runs": 1 },
    "latest": { "run_id": "...", "status": "done", "display_name": "my-api", "...": "history item" },
    "previous": { "run_id": "...", "status": "done", "display_name": "my-api", "...": "history item" },
    "trends": [
      {
        "run_id": "0192f3a0-0000-7000-8000-000000000000",
        "created_at": "2026-10-08T05:11:22.123Z",
        "build_time_ms": null,
        "startup_time_ms": 1500,
        "memory_peak_bytes": 73400320,
        "cpu_average_percent": 12.4,
        "requests_per_second": 121.45,
        "p95_ms": 8.7
      }
    ]
  }
}
```

| Field | Rule |
| --- | --- |
| `project` | the project shown. `name` is the saved target name of its newest run, otherwise the last folder of the path. `runtime` is the runtime of the newest run that has one, `source` is `target` or `custom`. `null` only when no run is stored at all. A path without runs is answered with its path and name, `runtime` and `source` `null` and zero totals |
| `projects` | every project that has a run, newest activity first, for the switcher. `[]` when no run is stored |
| `totals` | how the runs of this project ended. `total_runs` is `done_runs + failed_runs + cancelled_runs` |
| `latest` | the newest `done` run of the project, in the summary shape of an item of `GET /benchmarks`. `null` when it has no done run |
| `previous` | the done run before `latest`, so the changes can be shown without a second call. `null` when there is no second done run |
| `trends` | the newest 30 `done` runs of the project, newest first. `[]` when it has none |

A failed or cancelled run is counted in `totals` and in `projects`, but it is never `latest`, `previous` or a trend point, because its numbers are not a complete benchmark.

### Trend metric rules

Trend values follow the same rules as [GET /comparisons/summary](#get-comparisonssummary). A value that is unavailable, did not succeed or does not apply is `null`, never `0`. A `0` is an actual measured zero.

| Trend field | Present when | Otherwise |
| --- | --- | --- |
| `build_time_ms` | the build status is `success` | `null` |
| `startup_time_ms` | the startup status is `success` | `null` |
| `memory_peak_bytes` | the run has a memory result | `null` |
| `cpu_average_percent` | the run has a CPU result | `null` |
| `requests_per_second` | the HTTP status is `completed` and at least one request was observed | `null` |
| `p95_ms` | the HTTP status is `completed` and at least one request was observed | `null` |

- A build or startup with any status other than `success` (for example `no_build_command` or `no_start_command` on a done run) has no duration in the trend.
- Memory and CPU have no status, so they are present whenever they are stored. They are `null` for a run without a workload.
- Nothing is averaged, interpolated, rounded or otherwise transformed. Values are the stored values.

### Difference from the history list

`GET /benchmarks` reports the raw stored measurement values. The `trends` of the overview follow the comparison rules above instead, so the same measurement can be a number in `latest.summary` and `null` in `trends`. This is expected.

### Errors

| Case | Response |
| --- | --- |
| `target_path` is empty | 400 `VALIDATION_ERROR` |
| a method other than `GET` | 405 `METHOD_NOT_ALLOWED` |
| storage failure | 500 `INTERNAL_SERVER_ERROR` |

## GET /benchmarks/stage-averages

How long each stage usually takes for one project, for the estimate of a new run and the `+3 s vs last` hint of a live run.

```text
GET /api/v1/benchmarks/stage-averages?target_path=<path>
```

`target_path` is required and not empty, trailing slashes are ignored. The endpoint reads the newest 3 `done` runs of that path that are not deleted.

```json
{
  "success": true,
  "httpCode": 200,
  "message": "Stage averages retrieved.",
  "data": {
    "runs_used": 3,
    "stages": [
      { "stage": "build", "average_ms": 6496, "last_ms": 6100 },
      { "stage": "startup", "average_ms": 1500, "last_ms": null }
    ],
    "total_average_ms": 9350
  }
}
```

| Field | Rule |
| --- | --- |
| `runs_used` | 0 to 3 |
| `stages` | one entry per stage key, in run order. `average_ms` is averaged over the runs that have a duration for that stage, rounded to the nearest millisecond. `last_ms` is the duration in the newest run, `null` when that run has none. A stage without any duration is left out |
| `total_average_ms` | the average of the totals of the runs, each total being the sum of the stage durations of that run. `null` when no run has a duration |

With no done run the answer is `runs_used: 0`, `stages: []` and `total_average_ms: null`, and the UI shows `unknown`. A failed, cancelled or deleted run is never used.

| Case | Response |
| --- | --- |
| `target_path` missing or empty | 400 `VALIDATION_ERROR` |
| a method other than `GET` | 405 `METHOD_NOT_ALLOWED` |
| storage failure | 500 `INTERNAL_SERVER_ERROR` |

## GET /health

For the API pill in the top bar and the `API connected` footer. The UI measures the latency itself.

```json
{ "success": true, "httpCode": 200, "message": "The API is healthy.",
  "data": { "ok": true, "database": true, "version": "0.1.0" } }
```

When the database cannot be reached the answer is the error shape with `503 SERVICE_UNAVAILABLE`. Other methods answer `405`.

## GET /live-runs/active

The run that is executing now, for the `Run in progress` chip and to stop duplicate starts. `data` is `null` when nothing runs.

```json
{ "success": true, "httpCode": 200, "message": "Active run retrieved.",
  "data": {
    "run_id": "0192f3a0-0000-7000-8000-000000000000",
    "stage": "workload",
    "elapsed_ms": 18400,
    "workload_progress": { "mode": "requests", "done": 420, "total": 1000 }
  } }
```

| Field | Rule |
| --- | --- |
| `stage` | the stage started last, `null` before the first stage |
| `elapsed_ms` | milliseconds since the run was accepted, `null` for the instant the run is not registered yet |
| `workload_progress` | the newest progress of the workload, `{ "mode": "requests", "done", "total" }` or `{ "mode": "duration", "elapsed_ms", "total_ms" }`. `null` until the workload has sampled |

The run id is the same id that `GET /benchmarks/{run_id}` uses once the run is stored.

## GET /comparisons/summary

Compares two stored runs. Nothing is stored, the response is always calculated from the two historical runs.

```text
GET /api/v1/comparisons/summary?runIds=<baseline_run_id>,<comparison_run_id>
```

- The first id is the `baseline`, the second is the `comparison`. The order is never changed.
- `absolute_difference = comparison - baseline`
- `percentage_change = (comparison - baseline) / baseline * 100`, `null` when the baseline is `0`
- `direction` is `lower_is_better` or `higher_is_better`, decided by the backend
- Integer metrics (milliseconds, bytes, counts) stay exact integers, decimal metrics and percentages are rounded to 2 decimals
- A value that is unavailable on either side is `null`, and so are `absolute_difference` and `percentage_change`. A missing value is never treated as zero.

### Compared metrics

| Group | Metrics | Direction |
| --- | --- | --- |
| `build_time` | `duration_ms` | lower is better |
| `startup_time` | `duration_ms` | lower is better |
| `memory` | `baseline_rss_bytes`, `peak_rss_bytes`, `delta_rss_bytes` | lower is better |
| `cpu` | `baseline_percent`, `average_percent`, `peak_percent` | lower is better |
| `http_latency` | `requests_per_second` | higher is better |
| `http_latency` | `p50_ms`, `p95_ms`, `p99_ms` | lower is better |
| `artifact_size.source`, `.build`, `.excluded` | `bytes`, `file_count` | lower is better |
| `workspaces[]`, matched by `name` and `mode` | `image_bytes`, `build_duration_ms`, `startup_duration_ms`, `baseline_rss_bytes`, `peak_rss_bytes`, `delta_rss_bytes`, `cpu_baseline_percent`, `cpu_average_percent`, `cpu_peak_percent` | lower is better |

A workspace entry exists for every name and mode either run has. A side that did not have it, or whose mode did not succeed, has `null` values and no difference.

### When a metric counts as available

- Build and startup durations are compared only when that step's status is `success`.
- HTTP metrics are compared only when the HTTP status is `completed` and at least one request was observed. A `target_exited` run has partial numbers that are not a completed benchmark.
- Memory and CPU have no status and are compared whenever they are stored.
- `build_time`, `startup_time` and `http_latency` carry `baseline_status` and `comparison_status`, so a client can tell why a side is `null` (`no_build_command`, `no_start_command`, `failed`, `target_exited`, and so on). The HTTP statuses are `null` for a run without a workload.

### Response

```json
{
  "success": true,
  "httpCode": 200,
  "message": "Comparison completed.",
  "data": {
    "baseline": {
      "run_id": "...", "target": { "path": "/projects/api" },
      "runtime": "bun", "created_at": "2026-10-08T05:11:22.123Z"
    },
    "comparison": {
      "run_id": "...", "target": { "path": "/projects/api" },
      "runtime": "bun", "created_at": "2026-10-08T06:04:26.551Z"
    },
    "metrics": {
      "build_time": {
        "baseline_status": "success",
        "comparison_status": "success",
        "duration_ms": {
          "baseline_value": 6496,
          "comparison_value": 4872,
          "absolute_difference": -1624,
          "percentage_change": -25.0,
          "direction": "lower_is_better"
        }
      },
      "startup_time": { "baseline_status": "...", "comparison_status": "...", "duration_ms": {} },
      "memory": { "baseline_rss_bytes": {}, "peak_rss_bytes": {}, "delta_rss_bytes": {} },
      "cpu": { "baseline_percent": {}, "average_percent": {}, "peak_percent": {} },
      "http_latency": {
        "baseline_status": "completed", "comparison_status": "completed",
        "requests_per_second": {}, "p50_ms": {}, "p95_ms": {}, "p99_ms": {}
      },
      "artifact_size": {
        "source": { "bytes": {}, "file_count": {} },
        "build": { "bytes": {}, "file_count": {} },
        "excluded": { "bytes": {}, "file_count": {} }
      }
    }
  }
}
```

Every `{}` above is a metric object with the five fields `baseline_value`, `comparison_value`, `absolute_difference`, `percentage_change` and `direction`.

### Errors

| Case | Response |
| --- | --- |
| `runIds` missing or empty | 400 `VALIDATION_ERROR` |
| one id, or three or more ids | 400 `VALIDATION_ERROR` |
| the same id twice (letter case ignored) | 400 `VALIDATION_ERROR` |
| an id that is not a UUID, an empty segment, spaces around an id | 400 `VALIDATION_ERROR` |
| `runIds` given more than once | 400 `VALIDATION_ERROR` |
| either run does not exist (the message names every missing id) | 404 `NOT_FOUND` |
| storage failure | 500 `INTERNAL_SERVER_ERROR` |

Runs of different targets or runtimes can be compared, each side shows its own target and runtime.

A failed or cancelled run can be compared. Its parts that never finished are `null`, so those metrics have no difference. A deleted run is not found.

## GET /benchmarks/{run_id}/config

Returns the configuration a stored run executed with. `recorded` is `false` when no configuration row exists for the run, and `configuration` is then `null`. An unknown or deleted run returns `404 NOT_FOUND`, and a malformed run id returns `400 BAD_REQUEST`.

```json
{ "success": true, "httpCode": 200, "message": "Run configuration retrieved.",
  "data": { "run_id": "...", "recorded": true, "configuration": { "source": "target", "...": "..." } } }
```

Header names only are returned, never values. See [saved-targets.md](saved-targets.md) for the configuration shape.

## GET /benchmarks/configurations?limit=

The newest runs with their configuration, newest first, deleted runs left out. `limit` defaults to 50 and must be between 1 and 200, otherwise `400 VALIDATION_ERROR`. Each item has `run_id`, `created_at`, `recorded` and `configuration`.

## Request ids

The API gives every request a short id of 8 lowercase hex characters, returns it in the `x-request-id` header and, for an error, as `requestId` in the body. An id sent by the client is ignored. Every response with a status of 400 or more writes one line to the server log with the id, the method, the path, the status and the time taken, so an error shown in the UI can be searched in the logs.

```json
{ "success": false, "httpCode": 404, "code": "NOT_FOUND", "message": "...", "error": "...", "requestId": "a3f09c1e" }
```

## Error codes

| Code | Status |
| --- | --- |
| `BAD_REQUEST` | 400 |
| `VALIDATION_ERROR` | 400 |
| `FORBIDDEN` | 403 |
| `NOT_FOUND` | 404 |
| `METHOD_NOT_ALLOWED` | 405 |
| `CONFLICT` | 409 |
| `INTERNAL_SERVER_ERROR` | 500 |
| `SERVICE_UNAVAILABLE` | 503 |

## CORS

A browser may call the API from these origins: `http://localhost:5002` and `http://127.0.0.1:5002` (development), `http://localhost:8092` and `http://127.0.0.1:8092` (production), the dashboard at `http://localhost:5003` and `http://127.0.0.1:5003` (development) and `http://localhost:8093` and `http://127.0.0.1:8093` (production), and the tunnel hosts `*.usw3.devtunnels.ms` and `*.trycloudflare.com`. The same list protects the WebSocket handshake. Allowed methods are `GET`, `POST` and `DELETE`, the allowed request header is `Content-Type`, and `x-request-id` is exposed so a browser can read it.
