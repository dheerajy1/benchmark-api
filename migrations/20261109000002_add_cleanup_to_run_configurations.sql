-- The cleanup command the caller gave and how long it was allowed to run. Both are NULL for a
-- run without a cleanup, and for runs stored before this migration.
ALTER TABLE benchmark_run_configurations
    ADD COLUMN cleanup_command TEXT,
    ADD COLUMN cleanup_timeout_seconds BIGINT,
    ADD CONSTRAINT chk_benchmark_run_configurations_cleanup
        CHECK (
            (cleanup_command IS NULL AND cleanup_timeout_seconds IS NULL)
            OR (cleanup_command IS NOT NULL AND cleanup_timeout_seconds > 0)
        );

-- A run can now stop in the cleanup stage
ALTER TABLE benchmark_runs
    DROP CONSTRAINT ck_benchmark_runs_stopped_stage,
    ADD CONSTRAINT ck_benchmark_runs_stopped_stage
        CHECK (
            stopped_stage IS NULL OR stopped_stage IN (
                'discovery', 'artifact_size', 'build', 'startup',
                'baseline', 'workload', 'workspaces', 'cleanup', 'saving'
            )
        );
