CREATE TABLE IF NOT EXISTS enrollment_tokens (
    enrollment_token_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    token_hash TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at TEXT,
    revoked_at TEXT
);

CREATE TABLE IF NOT EXISTS device_enrollments (
    enrollment_id TEXT PRIMARY KEY,
    enrollment_token_id TEXT NOT NULL UNIQUE REFERENCES enrollment_tokens(enrollment_token_id),
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    local_provider_id TEXT,
    machine_id TEXT NOT NULL,
    public_key TEXT NOT NULL,
    key_algorithm TEXT NOT NULL,
    registration_payload_json TEXT NOT NULL,
    registration_payload_hash TEXT NOT NULL,
    hardware_fingerprint TEXT NOT NULL,
    agent_version TEXT NOT NULL,
    benchmark_version TEXT NOT NULL,
    nonce TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    status TEXT NOT NULL,
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    proof_attempts INTEGER NOT NULL DEFAULT 0,
    nonce_used_at TEXT,
    completed_at TEXT,
    device_id TEXT
);

CREATE TABLE IF NOT EXISTS device_credentials (
    credential_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    credential_hash TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    last_used_at TEXT,
    revoked_at TEXT
);

CREATE TABLE IF NOT EXISTS key_rotation_challenges (
    rotation_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    current_public_key_id TEXT NOT NULL REFERENCES provider_public_keys(public_key_id),
    new_public_key TEXT NOT NULL,
    key_algorithm TEXT NOT NULL,
    nonce TEXT NOT NULL,
    status TEXT NOT NULL,
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    proof_attempts INTEGER NOT NULL DEFAULT 0,
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_enrollment_tokens_provider
    ON enrollment_tokens(provider_id, status);
CREATE INDEX IF NOT EXISTS idx_device_enrollments_provider
    ON device_enrollments(provider_id, status);
CREATE INDEX IF NOT EXISTS idx_device_credentials_device
    ON device_credentials(device_id, status);
CREATE INDEX IF NOT EXISTS idx_key_rotation_device
    ON key_rotation_challenges(device_id, status);
CREATE UNIQUE INDEX IF NOT EXISTS idx_devices_provider_machine
    ON devices(provider_id, machine_id)
    WHERE machine_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_public_keys_active_device
    ON provider_public_keys(device_id)
    WHERE status = 'active';
CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_public_keys_active_value
    ON provider_public_keys(public_key)
    WHERE status = 'active';
