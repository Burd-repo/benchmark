CREATE TABLE IF NOT EXISTS job_leases (
    lease_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES compute_jobs(job_id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    schema_version TEXT NOT NULL,
    workload_type TEXT NOT NULL,
    gpu_uuid TEXT NOT NULL,
    policy_id TEXT,
    policy_version TEXT,
    status TEXT NOT NULL,
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    offered_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    accepted_at TEXT,
    provisioning_at TEXT,
    active_at TEXT,
    completed_at TEXT,
    failure_reason TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_job_leases_active_job
    ON job_leases(job_id)
    WHERE status IN ('offered', 'accepted', 'provisioning', 'active');
CREATE UNIQUE INDEX IF NOT EXISTS idx_job_leases_active_gpu
    ON job_leases(provider_id, device_id, gpu_uuid)
    WHERE status IN ('offered', 'accepted', 'provisioning', 'active');
CREATE INDEX IF NOT EXISTS idx_job_leases_provider_status
    ON job_leases(provider_id, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_job_leases_session_status
    ON job_leases(session_id, status, expires_at ASC);
CREATE INDEX IF NOT EXISTS idx_job_leases_job_status
    ON job_leases(job_id, status, updated_at DESC);