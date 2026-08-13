CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_id_organization
    ON projects(project_id, organization_id);

CREATE TABLE IF NOT EXISTS customer_artifacts (
    artifact_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    client_artifact_id TEXT,
    status TEXT NOT NULL,
    object_key TEXT NOT NULL UNIQUE,
    sha256 TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    content_type TEXT,
    upload_expires_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    verified_sha256 TEXT,
    verified_size_bytes BIGINT,
    uploaded_at TEXT,
    ready_at TEXT,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (project_id, organization_id)
        REFERENCES projects(project_id, organization_id),
    UNIQUE(project_id, client_artifact_id),
    UNIQUE(project_id, idempotency_key),
    CONSTRAINT customer_artifacts_status_allowed CHECK (
        status IN ('pending_upload', 'uploaded', 'ready', 'expired', 'rejected')
    ),
    CONSTRAINT customer_artifacts_sha256_format CHECK (
        sha256 ~ '^sha256:[a-f0-9]{64}$'
        AND (verified_sha256 IS NULL OR verified_sha256 ~ '^sha256:[a-f0-9]{64}$')
    ),
    CONSTRAINT customer_artifacts_size_bounds CHECK (
        size_bytes >= 0 AND size_bytes <= 10737418240
        AND (verified_size_bytes IS NULL OR (
            verified_size_bytes >= 0 AND verified_size_bytes <= 10737418240
        ))
    ),
    CONSTRAINT customer_artifacts_verification_pair CHECK (
        (verified_sha256 IS NULL AND verified_size_bytes IS NULL AND uploaded_at IS NULL)
        OR (verified_sha256 IS NOT NULL AND verified_size_bytes IS NOT NULL AND uploaded_at IS NOT NULL)
    ),
    CONSTRAINT customer_artifacts_ready_state CHECK (
        status <> 'ready'
        OR (ready_at IS NOT NULL AND verified_sha256 = sha256 AND verified_size_bytes = size_bytes)
    ),
    CONSTRAINT customer_artifacts_pending_state CHECK (
        status <> 'pending_upload'
        OR (verified_sha256 IS NULL AND verified_size_bytes IS NULL AND ready_at IS NULL)
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_customer_artifacts_project_binding
    ON customer_artifacts(artifact_id, project_id);
CREATE INDEX IF NOT EXISTS idx_customer_artifacts_project_status
    ON customer_artifacts(project_id, status, expires_at, created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_customer_workloads_project_binding
    ON customer_workloads(workload_id, project_id);

CREATE TABLE IF NOT EXISTS customer_workload_input_artifacts (
    workload_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    bound_at TEXT NOT NULL,
    PRIMARY KEY (workload_id, artifact_id),
    FOREIGN KEY (workload_id, project_id)
        REFERENCES customer_workloads(workload_id, project_id),
    FOREIGN KEY (artifact_id, project_id)
        REFERENCES customer_artifacts(artifact_id, project_id)
);

CREATE INDEX IF NOT EXISTS idx_customer_workload_input_artifacts_artifact
    ON customer_workload_input_artifacts(artifact_id, workload_id);
