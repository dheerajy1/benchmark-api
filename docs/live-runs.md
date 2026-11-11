# Live runs

A live run executes in the background. Its progress is stored as ordered events, and a client follows it over HTTP or a WebSocket. The run never depends on a client: it keeps going and is stored whether anybody watches, disconnects or reconnects.

Only one benchmark runs at a time. `POST /api/v1/live-runs` is the only way to start one.

Contents: [Endpoints](#endpoints), [Frames](#frames), [Client rules](#client-rules), [Close codes](#close-codes), [Origin and security](#origin-and-security), [Heartbeat and limits](#heartbeat-and-limits), [Samples](#samples), [Execution slot](#execution-slot), [Event log](#event-log), [Registry](#registry), [Run origin](#run-origin-and-the-custom-run-switch), [Cancelling](#cancelling-a-run).

## Endpoints

### `POST /api/v1/live-runs`

The request body is described in [api.md](api.md#starting-a-run). Validation and the path check happen first.

- `202`: the run exists and is registered.

```json
{ "success": true, "httpCode": 202, "message": "Benchmark accepted.",
  "data": { "run_id": "0192f3a0-...",
            "ws_path": "/api/v1/live-runs/0192f3a0-.../ws",
            "status_path": "/api/v1/live-runs/0192f3a0-...",
            "cancel_token": "3f9a0c1b7d2e48a6b5c4d3e2f1a09b8c",
            "capabilities": { "cancel": true } } }
```

- `400` for an invalid body or path.
- `409 CONFLICT` while a benchmark runs. `error` is the id of the active live run.

`run_id` is also the id of the stored run: every run that ends, completed, failed or cancelled, is stored under it.

### `GET /api/v1/live-runs/active`

The run that executes now, or `data: null` when nothing runs. See [api.md](api.md#get-live-runsactive). `cancel_token` is shown only here. It reads the slot and the registry entry of the run it names, never takes the slot and never changes a run. The slot always holds a live run, so the answer always has a run id, a stage and an elapsed time (`elapsed_ms` is `null` only in the instant before the run is registered).

### `GET /api/v1/live-runs/{run_id}?after=<seq>`

The JSON replay, the fallback when a WebSocket is not possible. `after` defaults to 0.

```json
{ "success": true, "httpCode": 200, "message": "Live run retrieved.",
  "data": { "run_id": "...", "status": "running", "last_seq": 17,
            "dropped_through_seq": 0, "truncated": false, "events": [ ... ] } }
```

`status` is `running`, `completed`, `failed` or `cancelled`. `404` for a run that is not in memory (unknown, or finished and expired): a finished and stored run is read with `GET /api/v1/benchmarks/{run_id}`, the stored run id is the live run id. `400` for a malformed id, `400 VALIDATION_ERROR` for a malformed `after`.

### `GET /api/v1/live-runs/{run_id}/ws?after=<seq>`

A WebSocket. The server only sends text frames, one JSON object each. A plain GET without the upgrade headers is not served as a WebSocket.

Before the upgrade, with a normal HTTP error body: `403 FORBIDDEN` for a refused origin, `400` for a malformed id or `after`.

### `POST /api/v1/live-runs/{run_id}/cancel`

See [Cancelling a run](#cancelling-a-run).

## Frames

First, always:

```json
{ "type": "stream.opened", "run_id": "...", "status": "running",
  "last_seq": 17, "truncated": false, "dropped_through_seq": 0 }
```

Then every stored event with `seq > after`, in order, and then new events as they happen. Each event has the envelope `{ v, run_id, seq, t_ms, at, type, data }`. The event types are `run.accepted`, `stage.*`, `measurement.available`, `baseline.measured`, `sample`, `workspace.phase`, `workspace.sample`, `cleanup.step`, `run.completed`, `run.failed`, `run.cancelling` and `run.cancelled`.

`run.accepted` also carries `source` (`custom` or `target`) and `target.id` and `target.name` (`null` for a custom run).

The three terminal events carry `persisted`: `true` once the run is stored (always for `run.completed`), `false` only when storing failed, and then `persist_error` says why. `run.failed` and `run.cancelled` also carry the `stage` the run stopped in, and the stored run is read with `GET /api/v1/benchmarks/{run_id}`.

When the run has workspaces, `run.accepted` carries `plan.workspaces` (their number), the `workspaces` stage runs after `workload`, `workspace.phase` reports `{workspace, mode, phase, status}` with `phase` one of `started`, `sampling`, `finished` (`status` only with `finished`), and `workspace.sample` carries `{workspace, mode, elapsed_ms, memory, cpu}` about every 500 ms while a mode is sampled, with the same `memory` and `cpu` shapes as `sample`. Like `sample`, `workspace.sample` events may be evicted from the log.

When the request has any cleanup command (`cleanup`, or `docker.cleanup` of a Docker workspace), `run.accepted` carries `plan.cleanup: true` (otherwise `false`) and a `cleanup` stage runs after the last measurement and before `saving`. `cleanup.step` reports each command: `{step, workspace, phase, status, duration_ms, message}` with `step` `build` or `docker`, `workspace` the workspace name for a `docker` step (otherwise `null`), `phase` `started` or `finished`, and `status` (`success`, `failed` or `timeout`), `duration_ms` and `message` (why it failed) only with `finished`. A failed cleanup does not fail the run.

When a client reads so slowly that samples were evicted before it read them, a control frame comes first:

```json
{ "type": "samples.dropped", "through_seq": 40 }
```

After the terminal event the server closes with code `1000`.

## Client rules

- Keep the last `seq`. Ignore an event with a `seq` at or below it.
- A gap in `seq` is legitimate only while the last `seq` is below the highest `dropped_through_seq` / `through_seq` announced; beyond it `seq` must be contiguous. Any other gap means reconnect with `after=<last seq>`.
- Reconnect with `after=<last seq>` after a lost connection. A finished run replays up to its terminal event and closes, so a late open recovers the final state.
- Close `4404`: the run is not in memory. Ask `GET /api/v1/benchmarks/{run_id}`.

## Close codes

| Code | Meaning |
|---|---|
| 1000 | The run reached its terminal event |
| 1001 | No pong within the timeout |
| 4404 | Unknown run (never existed, or expired from memory) |
| 4429 | Too many sockets on this run (8) or in total (32) |

A browser cannot read the status of a failed handshake, so an unknown run and a full run are accepted and closed with these codes instead of being refused.

## Origin and security

CORS does not apply to WebSockets, so the handshake checks the `Origin` header itself, with the same allow-list as CORS (`src/origin.rs`):

| Origin header | Result |
|---|---|
| absent | allowed: a browser always sends one, so the client is not a browser |
| on the allow-list | allowed |
| not on the list, `null`, or not valid text | `403` |
| more than one | `403` |

The origin is not authentication: anybody who can reach the API can start a run, as with every other request. Known limit of the allow-list: it accepts any `*.trycloudflare.com` and `*.usw3.devtunnels.ms` origin, so any tunnel of anybody passes. It lists the API's own local ports (5002 in development, 8092 in production) and the dashboard's (5003 in development, 8093 in production), on `localhost` and `127.0.0.1`.

## Heartbeat and limits

- The server pings every 20 s. At the first ping tick after 45 s without a pong, the socket is closed with `1001`.
- 8 sockets per run and 32 in total. A client frame larger than 1 KiB closes the socket; smaller frames are ignored.
- Every way a socket ends releases its place: close, drop, timeout, error.

## Samples

A run that somebody watches emits one `sample` event every 500 ms while the workload runs. A run nobody watches (the existing tests) measures and emits nothing extra: `ProgressSink::wants_samples()` is false for the no-op sink, so no gauge, counter or tick exists for it.

```json
{"type":"sample","data":{
  "workload_elapsed_ms": 2000,
  "memory": { "rss_bytes": 80123904, "peak_rss_bytes": 81920000 },
  "cpu": { "interval_percent": 41.23, "average_percent": 38.9, "peak_percent": 55.0 },
  "http": {
    "completed": 412, "successful": 412, "failed": 0, "requests_per_second": 206.0,
    "progress": { "mode": "requests", "done": 412, "total": 1000 }
  }
}}
```

- `progress` in duration mode: `{"mode":"duration","elapsed_ms":2000,"total_ms":30000}`.
- `memory` and `cpu` are `null` until a reading exists. Inside `cpu`, `interval_percent` is `null` when the interval was shorter than 50 ms and `peak_percent` is `null` until the first valid interval. `average_percent` is `null` until the first reading.
- `failed` is `completed - successful`. A snapshot never shows more successful than completed requests.
- Values are rounded to two decimals, like the final result.
- No latency statistic is ever part of a sample: p50, p95, p99, min, mean and max are computed from every request at the end and only appear in `run.completed`.

Where each value comes from:

| Value | Source |
|---|---|
| `memory.rss_bytes`, `peak_rss_bytes` | the existing 100 ms memory sampler, published on every reading |
| `cpu.interval_percent`, `peak_percent`, `average_percent` | the existing 100 ms CPU sampler, same formulas as the final result |
| `http.completed`, `successful` | two atomic counters the workload workers add to after every request |
| `http.requests_per_second` | `completed` over the workload time elapsed so far |
| `progress` | request mode: `completed` against the requested total; duration mode: elapsed time against the planned duration |

The samplers never emit events. The monitor loop of the workload executor, which already wakes every 25 ms to watch the target, builds and emits the sample, so no thread is added. The first sample comes at about 500 ms, so a workload shorter than that has none.

Differences from the final result: the last live `requests_per_second` divides by the time of the last sample, the final one by the full workload time including requests still in flight at the deadline, so the two can differ slightly. The final CPU and memory peaks include readings taken after the last sample.

## Execution slot

At most one benchmark executes at a time. `RunSlot` (`features/v1/benchmark/live/slot.rs`) is created once per router and held by `AppState.live`. `try_acquire` is atomic and returns a `RunPermit`, or the `ActiveRun` that holds the slot. A `RunPermit` can only be created by `try_acquire`, and `service::execute` and the `RunExecutor` trait take `&RunPermit`, so executing a benchmark without owning the slot does not compile. The permit-less `service::run` remains only as the original entry point for the existing tests; no handler calls it.

### Live submission (`LiveRuns::submit`)

| Step | What happens | Slot |
|---|---|---|
| 1 | `service::prepare` | free |
| 2 | `try_acquire(Live, run_id)`; busy returns `SubmitError::Busy` | taken |
| 3 | The run is registered and `run.accepted` (seq 1) is appended | held |
| 4 | A supervisor task is spawned and owns the permit | held |
| 5 | `spawn_blocking`: the permit moves in and comes back; a panic is caught inside | held |
| 6 | `repository.save_run`; a run that ended is stored whether it completed, failed or was cancelled, and only a successful save of a finished run leads to `run.completed` | held |
| 7 | The terminal event is appended, then the permit is dropped | released |

`RunOwnership` records a terminal `run.failed` in its `Drop` when none exists and only then releases the slot, so the slot is never free while its run is reported as running, including when the supervisor is dropped (runtime shutdown). Nothing a client does can stop a live run except a cancel request: no socket takes part in it.

## Event log

`EventLog` stores structural events (run, stage, measurement, terminal) without limit (bounded by construction to 64) and `sample` events in a ring of 1,200. One counter numbers both, so evicted samples leave holes in `seq`, and only there.

`replay(after)` returns the retained events with `seq > after` plus `last_seq`, the status, and `dropped_through_seq`, the seq of the newest evicted sample (0 when none). `truncated` is `dropped_through_seq > 0`. The client rule for gaps is in [Client rules](#client-rules). A terminal event moves the status in the same step, so nothing can follow it.

## Registry

`RunRegistry` keeps runs in memory. Finished runs are evicted after 10 minutes and beyond the newest 20; a run without a terminal event is never evicted, whatever the limits say.

## Run origin and the custom-run switch

Both submission paths resolve the incoming body first (`features/v1/targets/resolve.rs`). A raw request is refused with 403 while `ALLOW_CUSTOM_RUNS` is not `true`, and that check comes after the raw body has been parsed, so a malformed body still gets its 400. A saved-target body is resolved from the targets file and is never refused by the switch. The slot, the registry and the event log do not change. See [api.md](api.md#saved-targets) for saved targets.

## Cancelling a run

A cancelled run is stored like any other run, with the parts that finished and the stage where it stopped (`status: cancelled`). See [api.md](api.md#get-benchmarksrun_id).

### Token

`POST /api/v1/live-runs` answers `202` with `cancel_token`, 128 random bits as 32 hex digits read from the operating system. It is shown once, kept in memory only, and never part of an event, a replay, an error or a log line. Whoever has it may cancel that run. Watching a run needs no token, cancelling does.

`capabilities.cancel` is `true` in the `202` and in `run.accepted`.

### `POST /api/v1/live-runs/{run_id}/cancel`

Body: `{ "cancel_token": "<token>" }`. The token travels in the body so no CORS header is needed.

| Situation | Answer |
|---|---|
| First request on a running run | `202`, `{ "state": "cancelling", "already_requested": false }` |
| Duplicate while cancelling | `202`, `{ "state": "cancelling", "already_requested": true }` |
| Run already finished | `409 CONFLICT`, "The run already finished (completed, failed or cancelled)." |
| Run is saving its result | `409 CONFLICT`, "The run is already saving its result and can no longer be cancelled." |
| Unknown or expired run | `404` |
| Missing, empty or wrong token | `403 FORBIDDEN`, it never says which part was wrong and never repeats a token |
| Malformed run id or body | `400` |

The answer means the cancel request was taken, not that the run has stopped.

### Events and status

- `run.cancelling` `{ "stage": "<stage in progress>" }`: recorded once, when the request is taken. A normal event, so every watcher sees it. The run is still running.
- `run.cancelled` `{ "stage": "<last stage started>", "persisted": true }`: the terminal event. It is only recorded after the execution has returned, so its work and its cleanup are over, and after the run was stored. The stored run id is the live run id, so the cancelled banner can link to `GET /api/v1/benchmarks/{run_id}`. `persisted` is `false` only when storing failed, and then `persist_error` says why.
- Replay and `stream.opened` report `status: "cancelled"` for such a run.

### How a run ends: one atomic switch

Every run has a switch with three positions, moved only by compare-and-swap away from `Running`:

- `Running -> CancelRequested`: a cancel request got there first. The run ends as cancelled.
- `Running -> Finalizing`: the benchmark finished first. Cancelling is refused from then on and the result is saved.

Exactly one of the two can happen, so cancelling and finishing never both win.

| What the execution does | Terminal event |
|---|---|
| Returns cancelled | the run is stored as `cancelled`, then `run.cancelled` with `persisted: true` |
| Returns its result while a cancel request is pending | the finished work is stored as `cancelled`, then `run.cancelled` with `persisted: true` |
| Returns its result and no cancel request won | the result is stored as `done` or `failed` by its steps, then `run.completed` or `run.failed` |
| Fails on its own, even with a cancel request pending | the run is stored as `failed` with its message and the stage it stopped in, then `run.failed` with `persisted: true` |
| Panics | stored as `failed` without parts, then `run.failed` |

A run that stops is stored before its terminal event is recorded, with every part that finished and `null` for the rest. A stage that was running when it stopped is kept with the status `cancelled` or `failed`. The terminal event is recorded before the execution slot is released, as for every run.

### Other rules

- Closing the page or the WebSocket never cancels. Cancelling is only this request.
- A cancel request does not depend on any client being connected.
- A cancelled run is a finished run for retention: it expires like one and counts toward the cap of finished runs in memory.
- Cancelled and failed runs are in the history (`status` filter), in the counts and in the totals of the overview, but never as the latest run or a trend point of the overview, and Compare shows their unfinished parts as `null`.

### What the execution does when it is asked to stop

Each step looks at the request and stops what it started. "Latency" is the longest a step can take to notice.

| Step | How it stops | Latency |
|---|---|---|
| Between steps | checked before discovery, build, artifact size, startup and the workload | immediate |
| Build | the 20 ms wait loop sees the request, the build's whole process group is stopped and checked | about 20 ms plus the stop |
| Waiting for readiness | the 50 ms loop sees the request, each probe is capped at about 200 ms | about 250 ms plus the stop |
| Baseline | the stabilization wait is cut into 25 ms slices, the CPU baseline window is 100 ms, any sampler already started is stopped | about 100 ms |
| Workload | the 25 ms monitor loop sees the request, the target's group gets SIGKILL at once so requests in flight fail immediately, the workers are joined, the samplers are stopped | about 25 ms plus the join |
| Cleanup | the cleanup commands still run after a cancel, they are not cancellable and each stops at its own timeout | the command's own time |
| Saving | the point of no return, `409` | not cancellable |

### Stopping a process group, and checking it

Every build and every target runs in its own process group. Stopping one is checked for every run:

1. `SIGTERM` to the group, then up to 100 ms for it to exit.
2. `SIGKILL` to the group, then the group is looked at in `/proc` every 10 ms until no running process of it is left (a process that already ended and only waits to be collected does not count). `SIGKILL` is sent again every 500 ms while something is left.
3. If something is still running after 3 seconds, stopping failed.

Only then is the leader collected. The checks are the same on a normal ending, but a failure there is only a warning on stderr and the result stands.

`run.cancelled` is only recorded when the target and the build are really gone. If the group cannot be stopped, the run ends as `run.failed` (stored as `failed`) with a message naming the processes that are left, never as cancelled. The execution slot is still released, because holding it forever would block every later run.

On Linux this reads `/proc`. On a system without it the check cannot see anything and passes.
