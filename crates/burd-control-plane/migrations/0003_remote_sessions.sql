ALTER TABLE provider_sessions
    ADD COLUMN IF NOT EXISTS resume_token_hash TEXT,
    ADD COLUMN IF NOT EXISTS hardware_fingerprint TEXT,
    ADD COLUMN IF NOT EXISTS agent_version TEXT,
    ADD COLUMN IF NOT EXISTS capabilities_json TEXT NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS latest_report_hash TEXT,
    ADD COLUMN IF NOT EXISTS latest_challenge_id TEXT,
    ADD COLUMN IF NOT EXISTS heartbeat_interval_seconds INTEGER NOT NULL DEFAULT 15,
    ADD COLUMN IF NOT EXISTS missed_heartbeat_limit INTEGER NOT NULL DEFAULT 3,
    ADD COLUMN IF NOT EXISTS connection_id TEXT,
    ADD COLUMN IF NOT EXISTS connected_at TEXT,
    ADD COLUMN IF NOT EXISTS disconnected_at TEXT,
    ADD COLUMN IF NOT EXISTS degraded_at TEXT,
    ADD COLUMN IF NOT EXISTS revoked_at TEXT,
    ADD COLUMN IF NOT EXISTS disconnect_reason TEXT,
    ADD COLUMN IF NOT EXISTS updated_at TEXT;

CREATE TABLE IF NOT EXISTS session_heartbeats (
    heartbeat_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    sequence BIGINT NOT NULL,
    client_sent_at TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    sequence_gap BIGINT NOT NULL DEFAULT 0,
    payload_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    UNIQUE(session_id, sequence)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_sessions_active_device
    ON provider_sessions(device_id)
    WHERE status IN ('pending_connection', 'online', 'degraded', 'offline');
CREATE INDEX IF NOT EXISTS idx_provider_sessions_status_expiry
    ON provider_sessions(status, expires_at);
CREATE INDEX IF NOT EXISTS idx_session_heartbeats_session_time
    ON session_heartbeats(session_id, server_received_at);
