CREATE TABLE IF NOT EXISTS runtime_verification_challenges (
    challenge_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    gpu_uuid TEXT NOT NULL,
    runtime_backend TEXT NOT NULL,
    hardware_fingerprint TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('issued', 'acknowledged', 'verified', 'failed', 'expired')),
    nonce TEXT NOT NULL UNIQUE,
    challenge_json TEXT NOT NULL,
    verification_ttl_seconds INTEGER NOT NULL CHECK (verification_ttl_seconds BETWEEN 1 AND 604800),
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    acknowledged_at TEXT,
    submitted_at TEXT,
    verified_at TEXT,
    failed_at TEXT,
    expired_at TEXT,
    response_hash TEXT UNIQUE,
    public_key_id TEXT REFERENCES provider_public_keys(public_key_id),
    response_json TEXT,
    verification_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_runtime_verification_challenges_session_status
    ON runtime_verification_challenges(session_id, status, issued_at);
CREATE INDEX IF NOT EXISTS idx_runtime_verification_challenges_expiry
    ON runtime_verification_challenges(status, expires_at);
CREATE INDEX IF NOT EXISTS idx_runtime_verification_challenges_gpu
    ON runtime_verification_challenges(provider_id, device_id, gpu_uuid, status);

CREATE TABLE IF NOT EXISTS provider_runtime_verifications (
    verification_id TEXT PRIMARY KEY,
    challenge_id TEXT NOT NULL UNIQUE REFERENCES runtime_verification_challenges(challenge_id),
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    gpu_uuid TEXT NOT NULL,
    runtime_backend TEXT NOT NULL,
    hardware_fingerprint TEXT NOT NULL,
    runtime_verification_fingerprint TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('verified', 'superseded', 'expired')),
    verified_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    record_json TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_runtime_verifications_active_gpu
    ON provider_runtime_verifications(provider_id, device_id, gpu_uuid)
    WHERE status = 'verified';
CREATE INDEX IF NOT EXISTS idx_provider_runtime_verifications_fingerprint
    ON provider_runtime_verifications(runtime_verification_fingerprint, status);
CREATE INDEX IF NOT EXISTS idx_provider_runtime_verifications_expiry
    ON provider_runtime_verifications(status, expires_at);
