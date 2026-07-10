CREATE TABLE IF NOT EXISTS proof_challenges (
    challenge_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    status TEXT NOT NULL,
    nonce TEXT NOT NULL UNIQUE,
    schema_version TEXT NOT NULL,
    profile_version TEXT NOT NULL,
    required_fingerprint TEXT NOT NULL,
    required_gpu_uuid TEXT,
    required_backend TEXT NOT NULL,
    model_artifact_hash TEXT NOT NULL,
    prompt_seed TEXT NOT NULL,
    required_proofs_json TEXT NOT NULL,
    min_tokens_per_second DOUBLE PRECISION NOT NULL,
    max_ttft_ms BIGINT NOT NULL,
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    acknowledged_at TEXT,
    started_at TEXT,
    submitted_at TEXT,
    verified_at TEXT,
    failed_at TEXT,
    expired_at TEXT,
    response_hash TEXT UNIQUE,
    public_key_id TEXT REFERENCES provider_public_keys(public_key_id),
    response_object_key TEXT,
    response_json TEXT,
    verification_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_proof_challenges_session_status
    ON proof_challenges(session_id, status, issued_at);
CREATE INDEX IF NOT EXISTS idx_proof_challenges_provider_status
    ON proof_challenges(provider_id, status, issued_at);
CREATE INDEX IF NOT EXISTS idx_proof_challenges_expiry
    ON proof_challenges(status, expires_at);
CREATE INDEX IF NOT EXISTS idx_proof_challenges_gpu
    ON proof_challenges(required_gpu_uuid, status)
    WHERE required_gpu_uuid IS NOT NULL;
