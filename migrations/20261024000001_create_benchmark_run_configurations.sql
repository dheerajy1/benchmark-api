-- The configuration each run executed with, one row per run. Runs stored before this
-- migration have no row, and the API reports them as not recorded.
--
-- Header names only: header values are never stored.

CREATE TABLE benchmark_run_configurations (
    run_id UUID PRIMARY KEY,
    source TEXT NOT NULL,
    target_id TEXT,
    target_name TEXT,
    target_path TEXT NOT NULL,
    startup_program TEXT,
    startup_args TEXT[],
    readiness_type TEXT,
    readiness_host TEXT,
    readiness_port INTEGER,
    readiness_path TEXT,
    readiness_method TEXT,
    readiness_expected_status INTEGER,
    readiness_header_names TEXT[],
    workload_method TEXT,
    workload_path TEXT,
    workload_mode TEXT,
    workload_requests BIGINT,
    workload_duration_seconds BIGINT,
    workload_concurrency BIGINT,
    workload_header_names TEXT[],
    CONSTRAINT fk_benchmark_run_configurations_run_id
        FOREIGN KEY (run_id) REFERENCES benchmark_runs (id) ON DELETE CASCADE,
    CONSTRAINT chk_benchmark_run_configurations_source
        CHECK (source IN ('custom', 'target')),
    CONSTRAINT chk_benchmark_run_configurations_target
        CHECK (
            (source = 'target') = (target_id IS NOT NULL)
            AND (target_id IS NULL) = (target_name IS NULL)
        ),
    CONSTRAINT chk_benchmark_run_configurations_startup
        CHECK ((startup_program IS NULL) = (readiness_type IS NULL)),
    CONSTRAINT chk_benchmark_run_configurations_workload
        CHECK (
            workload_mode IS NULL
            OR (
                workload_mode IN ('requests', 'duration')
                AND workload_method IS NOT NULL
                AND workload_path IS NOT NULL
                AND workload_concurrency IS NOT NULL
                AND (
                    (workload_mode = 'requests'
                        AND workload_requests IS NOT NULL
                        AND workload_duration_seconds IS NULL)
                    OR (workload_mode = 'duration'
                        AND workload_duration_seconds IS NOT NULL
                        AND workload_requests IS NULL)
                )
            )
        )
);
