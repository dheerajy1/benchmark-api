# Execution guard and live-run core (Phase 1)

At most one benchmark executes at a time. Live runs (`POST /api/v1/live-runs`) are the only way to start one,
the old blocking `POST /api/v1/benchmarks` and its `Legacy` run kind were removed.

## Where the slot lives

`RunSlot` (`features/v1/benchmark/live/slot.rs`) is created once per router and held by
`AppState.live`. `try_acquire` is atomic and returns a `RunPermit`, or the `ActiveRun`
that holds the slot. A `RunPermit` can only be created by `try_acquire`, and
`service::execute` and the `RunExecutor` trait take `&RunPermit`, so executing a
benchmark without owning the slot does not compile. The permit-less `service::run`
remains only as the original entry point for the existing tests; no handler calls it.

`GET /api/v1/live-runs/active` reads the slot (`RunSlot::active`) and the registry entry of the run it names. It never
takes the slot and never changes a run, so it needs no guard. The slot always holds a live run, so the answer always has a
run id, a stage and an elapsed time (`elapsed_ms` is `null` only in the instant before the run is registered).

## Live submission (`LiveRuns::submit`)

| Step | What happens | Slot |
|---|---|---|
| 1 | `service::prepare` | free |
| 2 | `try_acquire(Live, run_id)`; busy returns `SubmitError::Busy` | taken |
| 3 | The run is registered and `run.accepted` (seq 1) is appended | held |
| 4 | A supervisor task is spawned and owns the permit | held |
| 5 | `spawn_blocking`: the permit moves in and comes back; a panic is caught inside | held |
| 6 | `repository.save_run`; a run that ended is stored whether it completed, failed or was cancelled, and only a successful save of a finished run leads to `run.completed` | held |
| 7 | The terminal event is appended, then the permit is dropped | released |

`RunOwnership` records a terminal `run.failed` in its `Drop` when none exists and only
then releases the slot, so the slot is never free while its run is reported as running,
including when the supervisor is dropped (runtime shutdown). Nothing a client does can
stop a live run: no socket or request takes part in it.

## Event log

`EventLog` stores structural events (run, stage, measurement, terminal) without limit
(bounded by construction to 64) and `sample` events in a ring of 1,200. One counter
numbers both, so evicted samples leave holes in `seq`, and only there.

`replay(after)` returns the retained events with `seq > after` plus `last_seq`, the
status, and `dropped_through_seq`, the seq of the newest evicted sample (0 when none).
`truncated` is `dropped_through_seq > 0`.

Client rule: a gap in `seq` is legitimate only while the last seen `seq` is below
`dropped_through_seq`; beyond it `seq` must be contiguous, and anything else means
reconnect with `after`. A seq at or below the last seen one is a duplicate.

A terminal event moves the status in the same step, so nothing can follow it.

## Registry

`RunRegistry` keeps runs in memory. Finished runs are evicted after 10 minutes and beyond
the newest 20; a run without a terminal event is never evicted, whatever the limits say.

## Run origin and the custom-run switch

Both submission paths resolve the incoming body first (`features/v1/targets/resolve.rs`). A raw request is refused with 403 while `ALLOW_CUSTOM_RUNS` is not `true`, and that check comes after the raw body has been parsed, so a malformed body still gets its 400. A saved-target body is resolved from the targets file and is never refused by the switch. The slot, the registry and the event log do not change.

`run.accepted` gains `source` (`custom` or `target`) and `target.id` and `target.name`, which are `null` for a custom run. Nothing else in the event changes.
