-- The workspaces a run was configured with, as a JSON list. Environment variable names only,
-- never a value. Runs stored before this migration get an empty list.

ALTER TABLE benchmark_run_configurations
    ADD COLUMN workspaces JSONB NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE benchmark_run_configurations
    ADD CONSTRAINT chk_benchmark_run_configurations_workspaces
        CHECK (jsonb_typeof(workspaces) = 'array');
