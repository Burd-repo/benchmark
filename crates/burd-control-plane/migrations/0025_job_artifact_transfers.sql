CREATE TABLE IF NOT EXISTS job_artifact_uploads (
    job_id TEXT NOT NULL REFERENCES compute_jobs(job_id) ON DELETE CASCADE,
    artifact_id TEXT NOT NULL,
    object_key TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    content_type TEXT,
    uploaded_at TEXT NOT NULL,
    PRIMARY KEY(job_id, artifact_id)
);

CREATE INDEX IF NOT EXISTS idx_job_artifact_uploads_job
    ON job_artifact_uploads(job_id, uploaded_at DESC);
