ALTER TABLE evidence_records
    ADD COLUMN IF NOT EXISTS session_id TEXT REFERENCES provider_sessions(session_id),
    ADD COLUMN IF NOT EXISTS public_key_id TEXT REFERENCES provider_public_keys(public_key_id),
    ADD COLUMN IF NOT EXISTS report_hash TEXT,
    ADD COLUMN IF NOT EXISTS hardware_fingerprint TEXT,
    ADD COLUMN IF NOT EXISTS signed_at TEXT,
    ADD COLUMN IF NOT EXISTS issued_at TEXT,
    ADD COLUMN IF NOT EXISTS subject_id TEXT,
    ADD COLUMN IF NOT EXISTS revoked_at TEXT,
    ADD COLUMN IF NOT EXISTS revocation_reason TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_evidence_records_evidence_hash
    ON evidence_records(evidence_hash);
CREATE INDEX IF NOT EXISTS idx_evidence_records_provider_type_received
    ON evidence_records(provider_id, evidence_type, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_evidence_records_status_expiry
    ON evidence_records(status, expires_at);
CREATE INDEX IF NOT EXISTS idx_evidence_records_report_hash
    ON evidence_records(report_hash);
CREATE INDEX IF NOT EXISTS idx_evidence_records_session
    ON evidence_records(session_id, server_received_at DESC);