-- The build command the caller gave and the build output directory they named.
-- Runs stored before this migration have no build, so both columns can be NULL.
-- An output directory of NULL means the runtime default was measured.

ALTER TABLE benchmark_run_configurations
    ADD COLUMN build_command TEXT,
    ADD COLUMN build_output_dir TEXT,
    ADD CONSTRAINT chk_benchmark_run_configurations_build
        CHECK (build_command IS NOT NULL OR build_output_dir IS NULL);
