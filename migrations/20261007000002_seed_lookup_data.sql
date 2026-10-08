-- Stable lookup rows, ids are fixed UUIDv7 values and never regenerated.
-- Application code resolves these rows by code, not by id.

INSERT INTO runtimes (id, code, name) VALUES
    ('01a11681-58ce-7062-be34-109f4dcae7e8', 'bun', 'Bun'),
    ('01a11681-58cf-7458-8bfd-90172fda432b', 'node', 'Node.js'),
    ('01a11681-58d0-73ab-984d-77030b780d4e', 'python', 'Python'),
    ('01a11681-58d1-7184-a78f-d4a284a6c3f2', 'rust', 'Rust'),
    ('01a11681-58d2-7807-91ed-66a3899f3298', 'go', 'Go')
ON CONFLICT (code) DO NOTHING;

INSERT INTO artifact_types (id, code, name) VALUES
    ('01a11681-58d3-7263-a328-73be5e8a9c83', 'source', 'Source'),
    ('01a11681-58d4-7bb1-8261-943fc0899449', 'build', 'Build'),
    ('01a11681-58d5-706a-8c3a-5a258650d32e', 'excluded', 'Excluded')
ON CONFLICT (code) DO NOTHING;

INSERT INTO benchmark_statuses (id, code, name) VALUES
    ('01a11681-58d6-7c8f-9348-95ff00100ffe', 'success', 'Success'),
    ('01a11681-58d7-7a9a-98aa-361fa648aa65', 'failed', 'Failed'),
    ('01a11681-58d8-777c-975c-cb08d824a458', 'timeout', 'Timeout'),
    ('01a11681-58d9-7763-a36d-5e784b38bfbb', 'unhealthy', 'Unhealthy'),
    ('01a11681-58da-7be5-b310-1d15f690a0ff', 'no_build_command', 'No build command'),
    ('01a11681-58db-7735-a839-1199baac704f', 'no_start_command', 'No start command'),
    ('01a11681-58dc-73d6-9b0d-e01f42d9efd7', 'no_port', 'No port'),
    ('01a11681-58dd-7cdb-b8a1-5c6fa9b94057', 'completed', 'Completed'),
    ('01a11681-58de-7a0e-8e07-cfb20f7502a2', 'target_exited', 'Target exited')
ON CONFLICT (code) DO NOTHING;
