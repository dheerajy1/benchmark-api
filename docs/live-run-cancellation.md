# Cancelling a live run (control plane, C1)

This document describes the cancellation contract and the control plane that decides how a run ends.
What the execution does when it is asked to stop (build, startup, workload, samplers, process groups)
comes in the next step, see "Not yet" at the end.

## Token

`POST /api/v1/live-runs` answers `202` with `cancel_token`, 128 random bits as 32 hex digits read from the
operating system. It is shown once, kept in memory only, and never part of an event, a replay, an error or a
log line. Whoever has it may cancel that run. Watching a run needs no token, cancelling does.

`capabilities.cancel` is `true` in the `202` and in `run.accepted`.

## `POST /api/v1/live-runs/{run_id}/cancel`

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

## Events and status

- `run.cancelling` `{ "stage": "<stage in progress>" }`: recorded once, when the request is taken. A normal event,
  so every watcher sees it. The run is still running.
- `run.cancelled` `{ "stage": "<last stage started>", "persisted": false }`: the terminal event. It is only recorded
  after the execution has returned, so its work and its cleanup are over. Nothing is saved.
- Replay and `stream.opened` report `status: "cancelled"` for such a run.

## How a run ends: one atomic switch

Every run has a switch with three positions, moved only by compare-and-swap away from `Running`:

- `Running -> CancelRequested`: a cancel request got there first. The run ends as cancelled.
- `Running -> Finalizing`: the benchmark finished first. Cancelling is refused from then on and the result is saved.

Exactly one of the two can happen, so cancelling and finishing never both win.

| What the execution does | Terminal event |
|---|---|
| Returns cancelled | `run.cancelled` |
| Returns its result while a cancel request is pending | `run.cancelled`, the result is thrown away and not saved |
| Returns its result and no cancel request won | the result is saved, then `run.completed` |
| Fails on its own, even with a cancel request pending | `run.failed`, because that is what happened |
| Panics | `run.failed` |

The save is never attempted for a cancelled run. The terminal event is recorded before the execution slot is
released, as for every run.

## Other rules

- Closing the page or the WebSocket never cancels. Cancelling is only this request.
- A cancel request does not depend on any client being connected.
- A cancelled run is a finished run for retention: it expires like one and counts toward the cap of finished runs.
- Cancelled runs are not stored, there is no change to the database, the history or the other endpoints.

## What the execution does when it is asked to stop

Each step looks at the request and stops what it started. "Latency" is the longest a step can take to notice.

| Step | How it stops | Latency |
|---|---|---|
| Between steps | checked before discovery, artifact size, build, startup and the workload | immediate |
| Build | the 20 ms wait loop sees the request, the build's whole process group is stopped and checked | about 20 ms plus the stop |
| Waiting for readiness | the 50 ms loop sees the request, each probe is capped at about 200 ms | about 250 ms plus the stop |
| Baseline | the stabilization wait is cut into 25 ms slices, the CPU baseline window is 100 ms, any sampler already started is stopped | about 100 ms |
| Workload | the 25 ms monitor loop sees the request, the target's group gets SIGKILL at once so requests in flight fail immediately, the workers are joined, the samplers are stopped | about 25 ms plus the join |
| Saving | the point of no return, `409` | not cancellable |

## Stopping a process group, and checking it

Every build and every target runs in its own process group. Stopping one is now checked, for every run, live or legacy:

1. `SIGTERM` to the group, then up to 100 ms for it to exit.
2. `SIGKILL` to the group, then the group is looked at in `/proc` every 10 ms until no running process of it is left
   (a process that already ended and only waits to be collected does not count). `SIGKILL` is sent again every 500 ms
   while something is left.
3. If something is still running after 3 seconds, stopping failed.

Only then is the leader collected. The checks are the same on a normal ending, but a failure there is only a warning on
stderr and the result stands, as before.

`run.cancelled` is only recorded when the target and the build are really gone. If the group cannot be stopped, the run
ends as `run.failed` with a message naming the processes that are left, never as cancelled. The execution slot is still
released, because holding it forever would block every later run.

On Linux this reads `/proc`. On a system without it the check cannot see anything and passes.

## Verified in a real browser, and what was not

Observed on 2026-10-17 by hand, in one browser, with the dashboard served through a VS Code dev tunnel and this API behind a
Cloudflare quick tunnel, against the kafka-api-gateway project. After each cancel the server was checked for any process whose
working directory is the project and for a listener on the target's port.

Verified:

- **Cancel during the build:** the page showed Cancelled with "Stopped during the build step" and "nothing was stored". No process
  of the project was left and the target's port had no listener.
- **Cancel during the workload:** the events read "Workload started", "Cancellation requested, stopping Workload", "Run cancelled,
  nothing was stored". The target was stopped, no process was left and the port had no listener.
- **A second viewer:** a second tab opened on the same run showed "Only the tab that started this run can cancel it." After the
  original tab cancelled, both tabs showed the run as cancelled with nothing stored.

Not verified in a browser:

- **Cancel during startup.** The startup attempts ended on their own before a cancel could be sent: the target exited, and the API
  terminal showed a PostgreSQL host lookup error (`script "start" exited with code 1`). No cancellation during startup was seen.
- **Refresh, then cancel.** The run used for it failed at startup before it could be cancelled, so this is not established.
- **Double click on "Yes, cancel".** Not tried.
- Cancelling at the moment a run starts saving, a target whose processes cannot be stopped, other browsers, phones. These are
  covered by automated tests only, or not at all.
