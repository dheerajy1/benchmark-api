# Live runs API (Phase 3)

A live run executes in the background. Its progress is stored as ordered events, and a
client follows it over HTTP or a WebSocket. The run never depends on a client: it keeps
going and is stored whether anybody watches, disconnects or reconnects.

Only one benchmark runs at a time, shared with `POST /api/v1/benchmarks`
(see `execution-guard.md`).

## Endpoints

### `POST /api/v1/live-runs`

Same request body as `POST /api/v1/benchmarks`. Validation and the path check happen first.

- `202`: the run exists and is registered.

```json
{ "success": true, "httpCode": 202, "message": "Benchmark accepted.",
  "data": { "run_id": "0192f3a0-...",
            "ws_path": "/api/v1/live-runs/0192f3a0-.../ws",
            "status_path": "/api/v1/live-runs/0192f3a0-...",
            "capabilities": { "cancel": false } } }
```

- `400` for an invalid body or path, same errors as the legacy endpoint.
- `409 CONFLICT` while any benchmark runs. `error` is the id of the active live run, or
  "A benchmark is already running." when a legacy run holds the slot.

### `GET /api/v1/live-runs/{run_id}?after=<seq>`

The JSON replay, the fallback when a WebSocket is not possible. `after` defaults to 0.

```json
{ "success": true, "httpCode": 200, "message": "Live run retrieved.",
  "data": { "run_id": "...", "status": "running", "last_seq": 17,
            "dropped_through_seq": 0, "truncated": false, "events": [ ... ] } }
```

`status` is `running`, `completed` or `failed`. `404` for a run that is not in memory
(unknown, or finished and expired): a finished and stored run is read with
`GET /api/v1/benchmarks/{run_id}`. `400` for a malformed id, `400 VALIDATION_ERROR` for a
malformed `after`.

### `GET /api/v1/live-runs/{run_id}/ws?after=<seq>`

A WebSocket. The server only sends text frames, one JSON object each. A plain GET without
the upgrade headers is not served as a WebSocket.

Before the upgrade, with a normal HTTP error body: `403 FORBIDDEN` for a refused origin,
`400` for a malformed id or `after`.

## Frames

First, always:

```json
{ "type": "stream.opened", "run_id": "...", "status": "running",
  "last_seq": 17, "truncated": false, "dropped_through_seq": 0 }
```

Then every stored event with `seq > after`, in order, and then new events as they happen.
Each event has the envelope `{ v, run_id, seq, t_ms, at, type, data }`; the events and their
data are the contract of Phase 1 and 2 (`run.accepted`, `stage.*`, `measurement.available`,
`baseline.measured`, `sample`, `run.completed`, `run.failed`).

When a client reads so slowly that samples were evicted before it read them, a control
frame comes first:

```json
{ "type": "samples.dropped", "through_seq": 40 }
```

After the terminal event the server closes with code `1000`.

## Client rules

- Keep the last `seq`. Ignore an event with a `seq` at or below it.
- A gap in `seq` is legitimate only while the last `seq` is below the highest
  `dropped_through_seq` / `through_seq` announced; beyond it `seq` must be contiguous.
  Any other gap means reconnect with `after=<last seq>`.
- Reconnect with `after=<last seq>` after a lost connection. A finished run replays up to its
  terminal event and closes, so a late open recovers the final state.
- Close `4404`: the run is not in memory. Ask `GET /api/v1/benchmarks/{run_id}`.

## Close codes

| Code | Meaning |
|---|---|
| 1000 | The run reached its terminal event |
| 1001 | No pong within the timeout |
| 4404 | Unknown run (never existed, or expired from memory) |
| 4429 | Too many sockets on this run (8) or in total (32) |

A browser cannot read the status of a failed handshake, so an unknown run and a full run are
accepted and closed with these codes instead of being refused.

## Origin and security

CORS does not apply to WebSockets, so the handshake checks the `Origin` header itself, with
the same allow-list as CORS (`src/origin.rs`):

| Origin header | Result |
|---|---|
| absent | allowed: a browser always sends one, so the client is not a browser |
| on the allow-list | allowed |
| not on the list, `null`, or not valid text | `403` |
| more than one | `403` |

The origin is not authentication: anybody who can reach the API can start a run, as with
the legacy endpoint. Known limits of the allow-list, unchanged: it accepts any
`*.trycloudflare.com` and `*.usw3.devtunnels.ms` origin, so any tunnel of anybody passes, and it
lists only the dashboard's own local ports (5002 in development, 8092 in production), so a
dashboard served from another local port is refused.

## Heartbeat and limits

- The server pings every 20 s. At the first ping tick after 45 s without a pong, the socket
  is closed with `1001`.
- 8 sockets per run and 32 in total. A client frame larger than 1 KiB closes the socket;
  smaller frames are ignored.
- Every way a socket ends releases its place: close, drop, timeout, error.

## Not verified

No claim is made about Cloudflare Tunnel: idle timeouts, WebSocket support and ping behavior
through the actual tunnel have to be tested with a real run.
