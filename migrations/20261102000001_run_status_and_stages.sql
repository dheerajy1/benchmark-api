-- Run level status, soft delete and stage durations.
--
-- The database is dropped and created again before this runs, so no row has to be carried
-- over: the new columns are NOT NULL without a default and every insert sets them.

-- done: every step finished and none failed. failed: the run failed on its own, or a step that
-- ran has the status failed, timeout, unhealthy or target_exited. cancelled: stopped on request.
ALTER TABLE benchmark_runs
    ADD COLUMN status TEXT NOT NULL,
    -- The stage where a failed or cancelled run stopped, NULL for a done run
    ADD COLUMN stopped_stage TEXT,
    -- Why a failed run failed, NULL for done and cancelled runs
    ADD COLUMN failure_message TEXT,
    -- Soft delete: a row with a value here is hidden from every read
    ADD COLUMN deleted_at TIMESTAMPTZ;

ALTER TABLE benchmark_runs
    ADD CONSTRAINT ck_benchmark_runs_status
        CHECK (status IN ('done', 'failed', 'cancelled')),
    ADD CONSTRAINT ck_benchmark_runs_stopped_stage
        CHECK (
            stopped_stage IS NULL OR stopped_stage IN (
                'discovery', 'artifact_size', 'build', 'startup',
                'baseline', 'workload', 'workspaces', 'saving'
            )
        ),
    -- A done run has nothing that stopped it
    ADD CONSTRAINT ck_benchmark_runs_done_has_no_stop
        CHECK (status <> 'done' OR (stopped_stage IS NULL AND failure_message IS NULL)),
    -- Only a failed run has a failure message
    ADD CONSTRAINT ck_benchmark_runs_failure_message_only_failed
        CHECK (failure_message IS NULL OR status = 'failed'),
    ADD CONSTRAINT ck_benchmark_runs_failed_has_message
        CHECK (status <> 'failed' OR failure_message IS NOT NULL);

-- Every read hides deleted runs, so the indexes only hold the runs that are visible.
-- They replace the three indexes of the first schema.
DROP INDEX idx_benchmark_runs_created_at;
DROP INDEX idx_benchmark_runs_runtime_id_created_at;
DROP INDEX idx_benchmark_runs_target_path_created_at;

-- Serves "visible runs, newest first", the history list and its cursor
CREATE INDEX idx_benchmark_runs_visible_created_at
    ON benchmark_runs (created_at DESC, id DESC)
    WHERE deleted_at IS NULL;

-- Serves "visible runs of a runtime, newest first"
CREATE INDEX idx_benchmark_runs_visible_runtime_created_at
    ON benchmark_runs (runtime_id, created_at DESC)
    WHERE deleted_at IS NULL;

-- Serves "visible runs of a target, newest first"
CREATE INDEX idx_benchmark_runs_visible_target_created_at
    ON benchmark_runs (target_path, created_at DESC)
    WHERE deleted_at IS NULL;

-- Serves the done-only queries of one project: the overview and the stage averages
CREATE INDEX idx_benchmark_runs_visible_target_status_created_at
    ON benchmark_runs (target_path, status, created_at DESC)
    WHERE deleted_at IS NULL;

-- How long each stage of a run took. One row per stage that ran, in run order (position).
-- The stage is one of the 8 stage keys, or workspaces:<name>:native and
-- workspaces:<name>:docker for one mode of one workspace.
CREATE TABLE benchmark_run_stages (
    id UUID PRIMARY KEY,
    benchmark_run_id UUID NOT NULL,
    position INTEGER NOT NULL,
    stage TEXT NOT NULL,
    -- The status the stage ended with, cancelled or failed for the stage that was running
    -- when the run stopped
    status TEXT NOT NULL,
    -- NULL when the stage has no duration
    duration_ms BIGINT,
    CONSTRAINT fk_benchmark_run_stages_benchmark_run_id
        FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs (id) ON DELETE RESTRICT,
    -- Also serves lookups by benchmark_run_id (leading column)
    CONSTRAINT uq_benchmark_run_stages_run_stage UNIQUE (benchmark_run_id, stage),
    CONSTRAINT uq_benchmark_run_stages_run_position UNIQUE (benchmark_run_id, position),
    CONSTRAINT ck_benchmark_run_stages_stage_not_empty CHECK (stage <> ''),
    CONSTRAINT ck_benchmark_run_stages_position_non_negative CHECK (position >= 0),
    CONSTRAINT ck_benchmark_run_stages_duration_non_negative CHECK (duration_ms >= 0)
);

-- Serves "the durations of one stage across runs", the stage averages
CREATE INDEX idx_benchmark_run_stages_stage
    ON benchmark_run_stages (stage);
