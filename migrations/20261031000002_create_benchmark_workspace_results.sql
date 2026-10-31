-- What one mode (native or docker) of one workspace measured in a run, for example the
-- recordlog subscriber. One row per workspace and mode. A mode that could not be measured
-- keeps its status and NULL numbers.

CREATE TABLE benchmark_workspace_results (
    id UUID PRIMARY KEY,
    benchmark_run_id UUID NOT NULL,
    workspace_name TEXT NOT NULL,
    mode TEXT NOT NULL,
    status_id UUID NOT NULL,
    -- Why the mode failed, never holds an environment variable value
    message TEXT,
    -- Names only
    env_variable_names JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- Build step (docker always, native only with a build command)
    build_status_id UUID,
    build_duration_ms BIGINT,
    build_exit_code INTEGER,
    -- Docker only
    image_bytes BIGINT,
    startup_ms BIGINT,
    baseline_rss_bytes BIGINT,
    peak_rss_bytes BIGINT,
    delta_rss_bytes BIGINT,
    cpu_baseline_percent NUMERIC(10, 4),
    cpu_average_percent NUMERIC(10, 4),
    cpu_peak_percent NUMERIC(10, 4),
    load_status_id UUID,
    load_exit_code INTEGER,
    load_duration_ms BIGINT,
    -- Sampling window used when no load command ran
    idle_seconds BIGINT,
    CONSTRAINT fk_benchmark_workspace_results_benchmark_run_id
        FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs (id) ON DELETE RESTRICT,
    CONSTRAINT fk_benchmark_workspace_results_status_id
        FOREIGN KEY (status_id) REFERENCES benchmark_statuses (id) ON DELETE RESTRICT,
    CONSTRAINT fk_benchmark_workspace_results_build_status_id
        FOREIGN KEY (build_status_id) REFERENCES benchmark_statuses (id) ON DELETE RESTRICT,
    CONSTRAINT fk_benchmark_workspace_results_load_status_id
        FOREIGN KEY (load_status_id) REFERENCES benchmark_statuses (id) ON DELETE RESTRICT,
    -- Also serves lookups by benchmark_run_id (leading column)
    CONSTRAINT uq_benchmark_workspace_results_run_name_mode
        UNIQUE (benchmark_run_id, workspace_name, mode),
    CONSTRAINT ck_benchmark_workspace_results_mode CHECK (mode IN ('native', 'docker')),
    CONSTRAINT ck_benchmark_workspace_results_name_not_empty CHECK (workspace_name <> ''),
    CONSTRAINT ck_benchmark_workspace_results_names_array CHECK (
        jsonb_typeof(env_variable_names) = 'array'
    ),
    -- The build is either known or absent
    CONSTRAINT ck_benchmark_workspace_results_build_complete CHECK (
        num_nulls(build_status_id, build_duration_ms) IN (0, 2)
    ),
    CONSTRAINT ck_benchmark_workspace_results_build_exit_requires_build CHECK (
        build_exit_code IS NULL OR build_status_id IS NOT NULL
    ),
    -- Memory is all set or all NULL
    CONSTRAINT ck_benchmark_workspace_results_memory_complete CHECK (
        num_nulls(baseline_rss_bytes, peak_rss_bytes, delta_rss_bytes) IN (0, 3)
    ),
    -- CPU is all set or all NULL
    CONSTRAINT ck_benchmark_workspace_results_cpu_complete CHECK (
        num_nulls(cpu_baseline_percent, cpu_average_percent, cpu_peak_percent) IN (0, 3)
    ),
    -- A load result has both its status and its duration
    CONSTRAINT ck_benchmark_workspace_results_load_complete CHECK (
        num_nulls(load_status_id, load_duration_ms) IN (0, 2)
    ),
    CONSTRAINT ck_benchmark_workspace_results_load_exit_requires_load CHECK (
        load_exit_code IS NULL OR load_status_id IS NOT NULL
    ),
    CONSTRAINT ck_benchmark_workspace_results_numbers_non_negative CHECK (
        coalesce(build_duration_ms, 0) >= 0
        AND coalesce(image_bytes, 0) >= 0
        AND coalesce(startup_ms, 0) >= 0
        AND coalesce(baseline_rss_bytes, 0) >= 0
        AND coalesce(peak_rss_bytes, 0) >= 0
        AND coalesce(delta_rss_bytes, 0) >= 0
        AND coalesce(cpu_baseline_percent, 0) >= 0
        AND coalesce(cpu_average_percent, 0) >= 0
        AND coalesce(cpu_peak_percent, 0) >= 0
        AND coalesce(load_duration_ms, 0) >= 0
        AND coalesce(idle_seconds, 0) >= 0
    )
);

-- Serves "this workspace and mode across runs", the comparison and history questions
CREATE INDEX idx_benchmark_workspace_results_name_mode
    ON benchmark_workspace_results (workspace_name, mode);

CREATE INDEX idx_benchmark_workspace_results_status_id
    ON benchmark_workspace_results (status_id);
