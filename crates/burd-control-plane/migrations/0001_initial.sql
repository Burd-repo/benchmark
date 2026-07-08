CREATE TABLE IF NOT EXISTS schema_migrations (
    version TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
    user_id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS providers (
    provider_id TEXT PRIMARY KEY,
    user_id TEXT REFERENCES users(user_id),
    display_name TEXT,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS devices (
    device_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    machine_id TEXT,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provider_identities (
    identity_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT REFERENCES devices(device_id),
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    revoked_at TEXT
);

CREATE TABLE IF NOT EXISTS provider_public_keys (
    public_key_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT REFERENCES devices(device_id),
    public_key TEXT NOT NULL,
    key_algorithm TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    revoked_at TEXT
);

CREATE TABLE IF NOT EXISTS hardware_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT REFERENCES devices(device_id),
    hardware_fingerprint TEXT NOT NULL,
    report_hash TEXT,
    payload_json TEXT NOT NULL,
    observed_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS evidence_records (
    evidence_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT REFERENCES devices(device_id),
    evidence_type TEXT NOT NULL,
    canonicalization_version TEXT NOT NULL,
    evidence_hash TEXT NOT NULL,
    object_key TEXT,
    status TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    expires_at TEXT,
    verification_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provider_sessions (
    session_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT REFERENCES devices(device_id),
    status TEXT NOT NULL,
    sequence_last BIGINT NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL,
    last_seen_at TEXT,
    expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_events (
    audit_event_id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    actor_type TEXT NOT NULL,
    actor_id TEXT,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    idempotency_key TEXT,
    summary TEXT NOT NULL,
    metadata_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS idempotency_keys (
    scope TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    status_code INTEGER NOT NULL,
    response_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (scope, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_devices_provider_id ON devices(provider_id);
CREATE INDEX IF NOT EXISTS idx_provider_public_keys_provider_id ON provider_public_keys(provider_id);
CREATE INDEX IF NOT EXISTS idx_hardware_snapshots_provider_id ON hardware_snapshots(provider_id);
CREATE INDEX IF NOT EXISTS idx_evidence_records_provider_id ON evidence_records(provider_id);
CREATE INDEX IF NOT EXISTS idx_provider_sessions_provider_id ON provider_sessions(provider_id);
CREATE INDEX IF NOT EXISTS idx_audit_events_entity ON audit_events(entity_type, entity_id);