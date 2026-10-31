-- Unpacked size of the Docker image on disk, as `docker image ls` lists it. NULL for native
-- modes and for a Docker that does not report it.

ALTER TABLE benchmark_workspace_results
    ADD COLUMN image_disk_usage_bytes BIGINT;

ALTER TABLE benchmark_workspace_results
    ADD CONSTRAINT ck_benchmark_workspace_results_image_disk_usage_non_negative CHECK (
        coalesce(image_disk_usage_bytes, 0) >= 0
    );
