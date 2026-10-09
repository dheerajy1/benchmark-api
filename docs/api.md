# API reference

Base path: `/api/v1`. Every response uses the shapes in [architecture.md](architecture.md#response-shapes).

## POST /benchmarks

Runs a benchmark, stores it and returns the result.

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

## GET /benchmarks

Returns historical **summaries**, newest first.

| Parameter | Rule |
| --- | --- |
| `limit` | integer 1 to 100, default 20 |
| `before` | opaque cursor from a previous response |
| `runtime` | runtime lookup code such as `bun`, `node`, `python`, `rust`, `go` |
| `target_path` | exact match on the stored target path |

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
        "target": { "path": "/home/me/projects/my-api" },
        "discovery": { "runtime": "bun" },
        "created_at": "2026-10-08T05:11:22.123Z",
        "summary": {
          "build_time_ms": 1842,
          "startup_time_ms": 1500,
          "memory_peak_bytes": 73400320,
          "cpu_average_percent": 12.4,
          "requests_per_second": 121.45,
          "p95_ms": 8.7
        }
      }
    ],
    "pagination": { "limit": 20, "has_more": true, "next_cursor": "MTc5..." }
  }
}
```

- Every `summary` field is `null` when the run did not produce it. `discovery.runtime` is `null` when no runtime was detected.
- `next_cursor` is `null` on the last page.
- Treat the cursor as opaque, send it back unchanged as `before`.
- Runs are ordered by `created_at DESC, id DESC`, so runs with the same timestamp keep a stable order across pages.

### Errors

| Case | Response |
| --- | --- |
| `limit` is 0, negative, above 100 or not an integer | 400 `VALIDATION_ERROR` |
| `before` is not a cursor from this API | 400 `VALIDATION_ERROR` |
| `runtime` is empty or not a known runtime code | 400 `VALIDATION_ERROR` |
| `target_path` is empty | 400 `VALIDATION_ERROR` |
| storage failure | 500 `INTERNAL_SERVER_ERROR` |

## GET /benchmarks/{run_id}

Returns the same `data` as the POST that created the run, rebuilt from the normalized tables.

| Case | Response |
| --- | --- |
| run exists | 200 with the full benchmark |
| valid UUID, no such run | 404 `NOT_FOUND` |
| `run_id` is not a UUID | 400 `BAD_REQUEST` |

`discovery.build` is always `null`, as it is in the POST result.

## GET /benchmarks/overview

Returns the dashboard aggregate: the size of the whole history, the newest run and a trend over the newest runs. Everything is calculated from the stored runs on each request, nothing is stored or cached for it.

```text
GET /api/v1/benchmarks/overview
```

The endpoint has no query parameters. Any that are sent are ignored.

```json
{
  "success": true,
  "httpCode": 200,
  "message": "Benchmark overview retrieved.",
  "data": {
    "totals": { "total_runs": 128 },
    "latest": {
      "run_id": "0192f3a0-0000-7000-8000-000000000000",
      "target": { "path": "/home/me/projects/my-api" },
      "discovery": { "runtime": "bun" },
      "created_at": "2026-10-08T05:11:22.123Z",
      "summary": {
        "build_time_ms": 1842,
        "startup_time_ms": 1500,
        "memory_peak_bytes": 73400320,
        "cpu_average_percent": 12.4,
        "requests_per_second": 121.45,
        "p95_ms": 8.7
      }
    },
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
| `totals.total_runs` | the number of every persisted run. There is no run level status in the schema, so there is no `successful_runs` |
| `latest` | the newest run by `created_at DESC, id DESC`, the same order as `GET /benchmarks`, in the same summary shape as an item of that list. `null` when no run is stored |
| `trends` | the newest 30 runs of any kind, newest first, in the same order. `[]` when no run is stored. Runs are not filtered by success, because no run level success exists |

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

- A build or startup with any status other than `success` (for example `failed`, `timeout`, `no_build_command` or `no_start_command`) has no duration in the trend.
- An HTTP result with status `target_exited` has partial numbers that are not a completed benchmark, so both HTTP fields are `null`.
- Memory and CPU have no status, so they are present whenever they are stored. They are `null` for a run without a workload.
- Nothing is averaged, interpolated, rounded or otherwise transformed. Values are the stored values.

### Difference from the history list

`GET /benchmarks` is unchanged and still reports the raw stored measurement values, even when the status of that measurement is not a success. For example a `failed` build keeps its `build_time_ms` in the list.

The `trends` of the overview intentionally follow the comparison rules above instead. The same failed build is `null` in `trends`.

`latest` is the history summary of the newest run, so it uses the raw history behavior. For a newest run with a failed build, `latest.summary.build_time_ms` is the stored duration while the first trend point shows `null` for the same run. This is expected.

### Errors

| Case | Response |
| --- | --- |
| a method other than `GET` | 405 `METHOD_NOT_ALLOWED` |
| storage failure | 500 `INTERNAL_SERVER_ERROR` |

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