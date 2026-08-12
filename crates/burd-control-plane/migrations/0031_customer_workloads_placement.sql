CREATE TABLE IF NOT EXISTS customer_workloads (
    workload_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    schema_version TEXT NOT NULL,
    client_workload_id TEXT,
    workload_type TEXT NOT NULL,
    requirements_json TEXT NOT NULL,
    parameters_json TEXT NOT NULL DEFAULT '{}',
    timeout_seconds INTEGER NOT NULL,
    status TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    job_id TEXT UNIQUE REFERENCES compute_jobs(job_id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(project_id, client_workload_id),
    UNIQUE(project_id, idempotency_key),
    CONSTRAINT customer_workloads_status_allowed CHECK (
        status IN ('queued', 'placed', 'placement_failed', 'cancelled')
    ),
    CONSTRAINT customer_workloads_job_state CHECK (
        (status = 'placed' AND job_id IS NOT NULL)
        OR (status <> 'placed' AND job_id IS NULL)
    )
);

CREATE TABLE IF NOT EXISTS workload_execution_profiles (
    workload_type TEXT PRIMARY KEY,
    template_id TEXT NOT NULL,
    image_ref TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CONSTRAINT workload_execution_profiles_status_allowed CHECK (
        status IN ('active', 'disabled')
    ),
    CONSTRAINT workload_execution_profiles_digest_pinned CHECK (
        image_ref ~ '@sha256:[A-Fa-f0-9]{64}$'
    )
);

CREATE TABLE IF NOT EXISTS compute_placements (
    placement_id TEXT PRIMARY KEY,
    workload_id TEXT NOT NULL UNIQUE REFERENCES customer_workloads(workload_id),
    schema_version TEXT NOT NULL,
    listing_id TEXT NOT NULL REFERENCES marketplace_listings(listing_id),
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    gpu_uuid TEXT NOT NULL,
    policy_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    status TEXT NOT NULL,
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    runtime_admission_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CONSTRAINT compute_placements_status_allowed CHECK (
        status IN ('selected', 'released')
    )
);

ALTER TABLE compute_jobs
    ADD COLUMN IF NOT EXISTS workload_id TEXT UNIQUE REFERENCES customer_workloads(workload_id),
    ADD COLUMN IF NOT EXISTS placement_id TEXT UNIQUE REFERENCES compute_placements(placement_id);

ALTER TABLE compute_jobs
    ADD CONSTRAINT compute_jobs_workload_placement_pair_check CHECK (
        (workload_id IS NULL AND placement_id IS NULL)
        OR (workload_id IS NOT NULL AND placement_id IS NOT NULL)
    );

CREATE INDEX IF NOT EXISTS idx_customer_workloads_project_status
    ON customer_workloads(project_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_compute_placements_supply
    ON compute_placements(provider_id, device_id, lower(gpu_uuid), created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_compute_placements_active_supply
    ON compute_placements(provider_id, device_id, lower(gpu_uuid))
    WHERE status = 'selected';
