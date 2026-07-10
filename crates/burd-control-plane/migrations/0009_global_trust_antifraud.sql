CREATE TABLE IF NOT EXISTS provider_trust_states (
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    status TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    trust_score DOUBLE PRECISION NOT NULL,
    risk_score DOUBLE PRECISION NOT NULL,
    reliability_score DOUBLE PRECISION,
    verification_status TEXT,
    remote_network_score DOUBLE PRECISION,
    evidence_count INTEGER NOT NULL DEFAULT 0,
    successful_challenge_count INTEGER NOT NULL DEFAULT 0,
    failed_challenge_count INTEGER NOT NULL DEFAULT 0,
    session_status TEXT,
    latest_gpu_uuid TEXT,
    hardware_fingerprint TEXT,
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (provider_id, device_id)
);

CREATE TABLE IF NOT EXISTS antifraud_events (
    event_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    event_type TEXT NOT NULL,
    severity TEXT NOT NULL,
    status TEXT NOT NULL,
    reason TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    occurrence_count INTEGER NOT NULL DEFAULT 1,
    UNIQUE(provider_id, device_id, event_type, reason)
);

CREATE INDEX IF NOT EXISTS idx_provider_trust_states_status_score
    ON provider_trust_states(status, trust_score DESC, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_provider_trust_states_provider
    ON provider_trust_states(provider_id, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_antifraud_events_provider_status
    ON antifraud_events(provider_id, status, severity, last_seen_at DESC);
CREATE INDEX IF NOT EXISTS idx_antifraud_events_type
    ON antifraud_events(event_type, severity, last_seen_at DESC);