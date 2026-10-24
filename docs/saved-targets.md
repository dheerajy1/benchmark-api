# Saved targets and the custom-run switch

A saved target is a named project configuration kept in a JSON file on the server. A run
can start from a target by id, and only the workload mode, workload amount and concurrency
can be changed per run.

## Settings

| Variable | Meaning | Default |
| --- | --- | --- |
| `ALLOW_CUSTOM_RUNS` | `true` accepts raw requests that name a path and a command. Any other value, or no value, refuses them with 403. | `false` |
| `TARGETS_FILE` | Absolute path of the targets file. Unset or empty means no saved targets. | unset |

Both are read once at startup. A value that is not a valid boolean, or a targets file that
does not load, stops the server with a message that names the setting.

## Targets file

```json
{
  "targets": {
    "kafka-api-gateway": {
      "name": "Kafka API Gateway",
      "path": "/srv/projects/kafka-api-gateway",
      "startup": {
        "command": { "program": "bun", "args": ["run", "start"] },
        "readiness": {
          "port": 5000,
          "path": "/api/v1/health",
          "expected_status": 200,
          "headers": {
            "X-Client-Id": "nextjs_frontend",
            "X-Client-Secret": { "env": "KAFKA_GATEWAY_CLIENT_SECRET" }
          }
        }
      },
      "workload": {
        "method": "GET",
        "path": "/api/v1/items",
        "duration_seconds": 30,
        "concurrency": 50,
        "headers": { "X-Client-Secret": { "env": "KAFKA_GATEWAY_CLIENT_SECRET" } }
      }
    }
  }
}
```

- Target ids: 1 to 63 lowercase letters, digits or hyphens, not starting with a hyphen.
- `path` must be absolute.
- `startup` and `workload` are optional. A target with a workload must set exactly one of `requests` and `duration_seconds`.
- A header value is either a literal string or `{ "env": "NAME" }`. The variable is read at startup. A missing variable stops the server and the message names the variable only.
- Readiness `host` defaults to `127.0.0.1`, `method` to `GET`. Without `headers` the server's health credentials are sent, as in a raw request.
- Unknown keys are rejected. Parse errors give the line and column only, because a serde message can quote a value.
- The file is checked with the same validation as a raw request. A target that fails validation stops the server at startup.
- Keep the file readable by its owner only (`chmod 600`). The server warns when it is readable by other users.

Targets are read at startup. Edit the file and restart the server to change them.

## Starting a run from a target

```json
{ "target_id": "kafka-api-gateway", "workload": { "duration_seconds": 60, "concurrency": 20 } }
```

- `workload` is optional. Without it the target's own workload runs.
- Inside `workload`, only `requests`, `duration_seconds` and `concurrency` are accepted. Giving `requests` or `duration_seconds` replaces the target's mode. Giving neither keeps the target's mode and amount.
- A run may not change `path`, `startup`, readiness, or any header. Those keys return `400 VALIDATION_ERROR` and the message names the key, never a value.
- A target without a workload refuses a `workload` override with `400 VALIDATION_ERROR`.
- An unknown id returns `404 NOT_FOUND`.

The same body works on `POST /api/v1/live-runs` and `POST /api/v1/benchmarks`.

## Raw requests

A body without `target_id` is a raw request, parsed exactly as before. With `ALLOW_CUSTOM_RUNS` not set to `true`, a valid raw request returns `403 FORBIDDEN`:

```json
{ "success": false, "httpCode": 403, "code": "FORBIDDEN",
  "message": "This request is not allowed.",
  "error": "Custom runs are disabled on this server, use a saved target." }
```

A body that is malformed (bad JSON, wrong content type, wrong field types, missing `path`) returns the same 400 as before, whichever setting is in use.

One difference: a raw body that contains a `target_id` key is treated as a saved-target body. No existing client sends that key.

## GET /api/v1/targets

```json
{
  "success": true,
  "httpCode": 200,
  "message": "Saved targets retrieved.",
  "data": {
    "custom_runs_allowed": false,
    "targets": [
      {
        "id": "kafka-api-gateway",
        "name": "Kafka API Gateway",
        "path": "/srv/projects/kafka-api-gateway",
        "startup": {
          "command": { "program": "bun", "args": ["run", "start"] },
          "readiness": {
            "host": "127.0.0.1", "port": 5000, "path": "/api/v1/health",
            "method": "GET", "expected_status": 200,
            "header_names": ["X-Client-Id", "X-Client-Secret"]
          }
        },
        "workload": {
          "method": "GET", "path": "/api/v1/items", "mode": "duration",
          "requests": null, "duration_seconds": 30, "concurrency": 50,
          "header_names": ["X-Client-Secret"]
        }
      }
    ]
  }
}
```

Header values are never returned. `header_names` is `null` on readiness when the server's health credentials are used.

## Live-run events

`run.accepted` carries `source` (`custom` or `target`) and `target.id` and `target.name`, which are `null` for a custom run. See [execution-guard.md](execution-guard.md).

## Security

- `ALLOW_CUSTOM_RUNS=false` stops clients from submitting arbitrary commands and paths. It does not stop a saved target's project from running its own build tooling. For example, a Rust project's `build.rs` runs when that target is built.
- Saved-target runs are not authentication. Anyone who can reach the API can start a saved target, and that consumes CPU, memory, the single execution slot, and up to 300 seconds of workload. Authentication is a separate, later step.
- Header values are kept in memory only for the outgoing request. They are never written to a response, an event, a log line, the database, or a `Debug` output.
- Only header names are stored with a run. The configuration row added later stores names, never values.
