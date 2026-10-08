-- Lookup (reference) tables

CREATE TABLE runtimes (
    id UUID PRIMARY KEY,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    CONSTRAINT uq_runtimes_code UNIQUE (code)
);

CREATE TABLE artifact_types (
    id UUID PRIMARY KEY,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    CONSTRAINT uq_artifact_types_code UNIQUE (code)
);

CREATE TABLE benchmark_statuses (
    id UUID PRIMARY KEY,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    CONSTRAINT uq_benchmark_statuses_code UNIQUE (code)
);

-- Root entity: one row per complete benchmark execution

CREATE TABLE benchmark_runs (
    id UUID PRIMARY KEY,
    target_path TEXT NOT NULL,
    -- NULL when runtime discovery did not detect a runtime
    runtime_id UUID,
    created_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT fk_benchmark_runs_runtime_id
        FOREIGN KEY (runtime_id) REFERENCES runtimes (id) ON DELETE RESTRICT
);

-- Serves "runs ordered by created_at"
CREATE INDEX idx_benchmark_runs_created_at
    ON benchmark_runs (created_at);

-- Serves "runs for a runtime ordered by created_at", also covers runtime_id alone
CREATE INDEX idx_benchmark_runs_runtime_id_created_at
    ON benchmark_runs (runtime_id, created_at DESC);

-- Serves "latest run for a target"
CREATE INDEX idx_benchmark_runs_target_path_created_at
    ON benchmark_runs (target_path, created_at DESC);

-- Artifact size: one row per artifact category per run

CREATE TABLE artifact_sizes (
    id UUID PRIMARY KEY,
    benchmark_run_id UUID NOT NULL,
    artifact_type_id UUID NOT NULL,
    bytes BIGINT NOT NULL,
    file_count BIGINT NOT NULL,
    CONSTRAINT fk_artifact_sizes_benchmark_run_id
        FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs (id) ON DELETE RESTRICT,
    CONSTRAINT fk_artifact_sizes_artifact_type_id
        FOREIGN KEY (artifact_type_id) REFERENCES artifact_types (id) ON DELETE RESTRICT,
    -- Also serves lookups by benchmark_run_id (leading column)
    CONSTRAINT uq_artifact_sizes_run_artifact_type
        UNIQUE (benchmark_run_id, artifact_type_id),
    CONSTRAINT ck_artifact_sizes_bytes_non_negative CHECK (bytes >= 0),
    CONSTRAINT ck_artifact_sizes_file_count_non_negative CHECK (file_count >= 0)
);

CREATE INDEX idx_artifact_sizes_artifact_type_id
    ON artifact_sizes (artifact_type_id);

-- Build result

CREATE TABLE benchmark_builds (
    id UUID PRIMARY KEY,
    benchmark_run_id UUID NOT NULL,
    status_id UUID NOT NULL,
    duration_ms BIGINT,
    program TEXT,
    -- Argument vector of the build command
    args JSONB,
    working_directory TEXT,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    exit_code INTEGER,
    CONSTRAINT fk_benchmark_builds_benchmark_run_id
        FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs (id) ON DELETE RESTRICT,
    CONSTRAINT fk_benchmark_builds_status_id
        FOREIGN KEY (status_id) REFERENCES benchmark_statuses (id) ON DELETE RESTRICT,
    -- One build result per run, also serves lookups by benchmark_run_id
    CONSTRAINT uq_benchmark_builds_benchmark_run_id UNIQUE (benchmark_run_id),
    -- A build either never ran (all NULL) or ran (everything but exit_code set)
    CONSTRAINT ck_benchmark_builds_execution_complete CHECK (
        num_nulls(program, args, working_directory, duration_ms, started_at, finished_at) IN (0, 6)
    ),
    -- An exit code only exists for a build that ran
    CONSTRAINT ck_benchmark_builds_exit_code_requires_execution CHECK (
        exit_code IS NULL OR program IS NOT NULL
    ),
    CONSTRAINT ck_benchmark_builds_duration_non_negative CHECK (duration_ms >= 0),
    CONSTRAINT ck_benchmark_builds_finished_after_started CHECK (finished_at >= started_at)
);

-- Startup result

CREATE TABLE benchmark_startups (
    id UUID PRIMARY KEY,
    benchmark_run_id UUID NOT NULL,
    status_id UUID NOT NULL,
    duration_ms BIGINT,
    program TEXT,
    -- Argument vector of the start command
    args JSONB,
    working_directory TEXT,
    started_at TIMESTAMPTZ,
    ready_at TIMESTAMPTZ,
    exit_code INTEGER,
    readiness_type TEXT,
    readiness_host TEXT,
    readiness_port INTEGER,
    readiness_path TEXT,
    readiness_method TEXT,
    readiness_expected_status INTEGER,
    CONSTRAINT fk_benchmark_startups_benchmark_run_id
        FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs (id) ON DELETE RESTRICT,
    CONSTRAINT fk_benchmark_startups_status_id
        FOREIGN KEY (status_id) REFERENCES benchmark_statuses (id) ON DELETE RESTRICT,
    -- One startup result per run, also serves lookups by benchmark_run_id
    CONSTRAINT uq_benchmark_startups_benchmark_run_id UNIQUE (benchmark_run_id),
    -- The command is either fully known or fully absent
    CONSTRAINT ck_benchmark_startups_command_complete CHECK (
        num_nulls(program, args) IN (0, 2)
    ),
    -- The readiness configuration is either fully known or fully absent
    CONSTRAINT ck_benchmark_startups_readiness_complete CHECK (
        num_nulls(
            readiness_type, readiness_host, readiness_port,
            readiness_path, readiness_method, readiness_expected_status
        ) IN (0, 6)
    ),
    -- A startup either never launched (no measurement) or launched (fully measured)
    CONSTRAINT ck_benchmark_startups_execution_complete CHECK (
        num_nulls(duration_ms, working_directory, started_at, readiness_type) IN (0, 4)
    ),
    -- A launched startup always has its command
    CONSTRAINT ck_benchmark_startups_execution_requires_command CHECK (
        duration_ms IS NULL OR program IS NOT NULL
    ),
    -- ready_at and exit_code only exist for a startup that launched
    CONSTRAINT ck_benchmark_startups_outcome_requires_execution CHECK (
        (ready_at IS NULL AND exit_code IS NULL) OR started_at IS NOT NULL
    ),
    CONSTRAINT ck_benchmark_startups_duration_non_negative CHECK (duration_ms >= 0),
    CONSTRAINT ck_benchmark_startups_ready_after_started CHECK (ready_at >= started_at),
    CONSTRAINT ck_benchmark_startups_readiness_port_range CHECK (
        readiness_port BETWEEN 1 AND 65535
    ),
    CONSTRAINT ck_benchmark_startups_readiness_expected_status_range CHECK (
        readiness_expected_status BETWEEN 100 AND 599
    )
);

-- Memory result (absent when no workload ran)

CREATE TABLE benchmark_memories (
    id UUID PRIMARY KEY,
    benchmark_run_id UUID NOT NULL,
    baseline_rss_bytes BIGINT NOT NULL,
    peak_rss_bytes BIGINT NOT NULL,
    delta_rss_bytes BIGINT NOT NULL,
    CONSTRAINT fk_benchmark_memories_benchmark_run_id
        FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs (id) ON DELETE RESTRICT,
    -- One memory result per run, also serves lookups by benchmark_run_id
    CONSTRAINT uq_benchmark_memories_benchmark_run_id UNIQUE (benchmark_run_id),
    CONSTRAINT ck_benchmark_memories_bytes_non_negative CHECK (
        baseline_rss_bytes >= 0 AND peak_rss_bytes >= 0 AND delta_rss_bytes >= 0
    )
);

-- CPU result (absent when no workload ran)

CREATE TABLE benchmark_cpus (
    id UUID PRIMARY KEY,
    benchmark_run_id UUID NOT NULL,
    baseline_percent NUMERIC(10, 4) NOT NULL,
    average_percent NUMERIC(10, 4) NOT NULL,
    peak_percent NUMERIC(10, 4) NOT NULL,
    CONSTRAINT fk_benchmark_cpus_benchmark_run_id
        FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs (id) ON DELETE RESTRICT,
    -- One CPU result per run, also serves lookups by benchmark_run_id
    CONSTRAINT uq_benchmark_cpus_benchmark_run_id UNIQUE (benchmark_run_id),
    CONSTRAINT ck_benchmark_cpus_percent_non_negative CHECK (
        baseline_percent >= 0 AND average_percent >= 0 AND peak_percent >= 0
    )
);

-- HTTP latency result (absent when no workload ran)

CREATE TABLE benchmark_http_latencies (
    id UUID PRIMARY KEY,
    benchmark_run_id UUID NOT NULL,
    status_id UUID NOT NULL,
    total_requests BIGINT NOT NULL,
    successful_requests BIGINT NOT NULL,
    failed_requests BIGINT NOT NULL,
    duration_ms BIGINT NOT NULL,
    requests_per_second NUMERIC(14, 4) NOT NULL,
    -- Latency statistics are NULL when no request was observed
    min_ms NUMERIC(14, 4),
    mean_ms NUMERIC(14, 4),
    p50_ms NUMERIC(14, 4),
    p95_ms NUMERIC(14, 4),
    p99_ms NUMERIC(14, 4),
    max_ms NUMERIC(14, 4),
    CONSTRAINT fk_benchmark_http_latencies_benchmark_run_id
        FOREIGN KEY (benchmark_run_id) REFERENCES benchmark_runs (id) ON DELETE RESTRICT,
    CONSTRAINT fk_benchmark_http_latencies_status_id
        FOREIGN KEY (status_id) REFERENCES benchmark_statuses (id) ON DELETE RESTRICT,
    -- One HTTP result per run, also serves lookups by benchmark_run_id
    CONSTRAINT uq_benchmark_http_latencies_benchmark_run_id UNIQUE (benchmark_run_id),
    CONSTRAINT ck_benchmark_http_latencies_counts CHECK (
        total_requests >= 0
        AND successful_requests >= 0
        AND failed_requests >= 0
        AND successful_requests + failed_requests = total_requests
    ),
    CONSTRAINT ck_benchmark_http_latencies_duration_non_negative CHECK (duration_ms >= 0),
    CONSTRAINT ck_benchmark_http_latencies_rate_non_negative CHECK (requests_per_second >= 0),
    CONSTRAINT ck_benchmark_http_latencies_statistics_complete CHECK (
        num_nulls(min_ms, mean_ms, p50_ms, p95_ms, p99_ms, max_ms) IN (0, 6)
    )
);
