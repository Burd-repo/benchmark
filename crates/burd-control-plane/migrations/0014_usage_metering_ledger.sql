CREATE TABLE IF NOT EXISTS usage_ledger_entries (
    entry_id TEXT PRIMARY KEY,
    schema_version TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    job_id TEXT NOT NULL REFERENCES compute_jobs(job_id),
    lease_id TEXT REFERENCES job_leases(lease_id),
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    workload_type TEXT NOT NULL,
    gpu_uuid TEXT NOT NULL,
    job_status TEXT NOT NULL,
    lease_started_at TEXT,
    lease_ended_at TEXT,
    job_started_at TEXT,
    job_completed_at TEXT,
    reserved_gpu_seconds BIGINT NOT NULL,
    actual_gpu_seconds BIGINT NOT NULL,
    billable_gpu_seconds BIGINT NOT NULL,
    non_billable_gpu_seconds BIGINT NOT NULL,
    idle_billable_gpu_seconds BIGINT NOT NULL,
    idle_unbillable_gpu_seconds BIGINT NOT NULL,
    input_bytes BIGINT NOT NULL,
    output_bytes BIGINT NOT NULL,
    network_transfer_bytes BIGINT NOT NULL,
    storage_bytes BIGINT NOT NULL,
    retry_count INTEGER NOT NULL,
    provider_caused_failure BOOLEAN NOT NULL,
    customer_caused_failure BOOLEAN NOT NULL,
    failure_classification TEXT,
    challenge_non_billable_seconds BIGINT NOT NULL,
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    receipt_json TEXT NOT NULL,
    receipt_hash TEXT NOT NULL,
    receipt_signature TEXT,
    receipt_public_key TEXT,
    receipt_signature_status TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(job_id, entry_type)
);

CREATE INDEX IF NOT EXISTS idx_usage_ledger_provider_time
    ON usage_ledger_entries(provider_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_usage_ledger_job_time
    ON usage_ledger_entries(job_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_usage_ledger_lease_time
    ON usage_ledger_entries(lease_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_usage_ledger_receipt_hash
    ON usage_ledger_entries(receipt_hash);

CREATE OR REPLACE FUNCTION prevent_usage_ledger_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'usage_ledger_entries is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS usage_ledger_no_update ON usage_ledger_entries;
CREATE TRIGGER usage_ledger_no_update
    BEFORE UPDATE OR DELETE ON usage_ledger_entries
    FOR EACH ROW EXECUTE FUNCTION prevent_usage_ledger_mutation();