CREATE TABLE IF NOT EXISTS compute_jobs (
    job_id TEXT PRIMARY KEY,
    client_job_id TEXT,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    schema_version TEXT NOT NULL,
    workload_type TEXT NOT NULL,
    template_id TEXT NOT NULL,
    image_ref TEXT NOT NULL,
    gpu_uuid TEXT NOT NULL,
    backend TEXT NOT NULL,
    parameters_json TEXT NOT NULL DEFAULT '{}',
    input_artifacts_json TEXT NOT NULL DEFAULT '[]',
    expected_outputs_json TEXT NOT NULL DEFAULT '[]',
    result_artifacts_json TEXT NOT NULL DEFAULT '[]',
    result_metrics_json TEXT NOT NULL DEFAULT '{}',
    policy_id TEXT,
    policy_version TEXT,
    status TEXT NOT NULL,
    progress_percent DOUBLE PRECISION,
    status_message TEXT,
    error_code TEXT,
    error_message TEXT,
    cancellation_reason TEXT,
    timeout_seconds INTEGER NOT NULL,
    job_credential_hash TEXT,
    job_credential_expires_at TEXT,
    created_at TEXT NOT NULL,
    assigned_at TEXT,
    accepted_at TEXT,
    started_at TEXT,
    completed_at TEXT,
    updated_at TEXT NOT NULL,
    UNIQUE(provider_id, client_job_id)
);

CREATE TABLE IF NOT EXISTS job_events (
    event_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES compute_jobs(job_id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    sequence BIGINT NOT NULL,
    schema_version TEXT NOT NULL,
    event_type TEXT NOT NULL,
    progress_percent DOUBLE PRECISION,
    message TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    occurred_at TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    UNIQUE(job_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_compute_jobs_provider_status
    ON compute_jobs(provider_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_compute_jobs_session_status
    ON compute_jobs(session_id, status, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_compute_jobs_workload_status
    ON compute_jobs(workload_type, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_job_events_job_sequence
    ON job_events(job_id, sequence);
CREATE INDEX IF NOT EXISTS idx_job_events_provider_time
    ON job_events(provider_id, server_received_at DESC);