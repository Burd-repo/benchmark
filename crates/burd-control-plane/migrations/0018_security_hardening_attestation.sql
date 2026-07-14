CREATE TABLE IF NOT EXISTS device_security_postures (
    posture_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    schema_version TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    status TEXT NOT NULL,
    posture_hash TEXT NOT NULL UNIQUE,
    public_key_id TEXT NOT NULL REFERENCES provider_public_keys(public_key_id),
    signature TEXT NOT NULL,
    canonicalization_version TEXT NOT NULL,
    agent_version TEXT NOT NULL,
    release_channel TEXT NOT NULL,
    key_storage_backend TEXT NOT NULL,
    key_hardware_backed BOOLEAN NOT NULL,
    private_key_exportable BOOLEAN NOT NULL,
    attestation_mode TEXT NOT NULL,
    attestation_evidence_hash TEXT,
    binary_hash TEXT,
    sbom_hash TEXT,
    vulnerability_scan_status TEXT NOT NULL,
    dependency_scan_status TEXT NOT NULL,
    hardware_fingerprint TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    verification_json TEXT NOT NULL,
    UNIQUE(provider_id, device_id, session_id, posture_hash)
);

CREATE INDEX IF NOT EXISTS idx_device_security_postures_provider_time
    ON device_security_postures(provider_id, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_device_security_postures_device_status
    ON device_security_postures(device_id, status, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_device_security_postures_session
    ON device_security_postures(session_id, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_device_security_postures_attestation
    ON device_security_postures(attestation_mode, status, server_received_at DESC);

CREATE OR REPLACE FUNCTION prevent_device_security_posture_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'device_security_postures is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS device_security_postures_no_update ON device_security_postures;
CREATE TRIGGER device_security_postures_no_update
    BEFORE UPDATE OR DELETE ON device_security_postures
    FOR EACH ROW EXECUTE FUNCTION prevent_device_security_posture_mutation();