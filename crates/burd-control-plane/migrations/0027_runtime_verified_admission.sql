ALTER TABLE provider_runtime_verifications
    ADD COLUMN IF NOT EXISTS public_key_id TEXT REFERENCES provider_public_keys(public_key_id),
    ADD COLUMN IF NOT EXISTS runtime_admission_fingerprint TEXT,
    ADD COLUMN IF NOT EXISTS runtime_admission_claims_json TEXT;

-- v1 records did not bind the signer key or carry the re-observable admission fingerprint.
-- They remain auditable but must never become admission-authoritative.
UPDATE provider_runtime_verifications
SET status = 'superseded'
WHERE status = 'verified'
  AND (
      public_key_id IS NULL
      OR runtime_admission_fingerprint IS NULL
      OR runtime_admission_claims_json IS NULL
  );

ALTER TABLE provider_runtime_verifications
    ADD CONSTRAINT provider_runtime_verifications_v2_verified_fields
    CHECK (
        status <> 'verified'
        OR (
            public_key_id IS NOT NULL
            AND runtime_admission_fingerprint IS NOT NULL
            AND runtime_admission_claims_json IS NOT NULL
        )
    );

CREATE TABLE IF NOT EXISTS provider_runtime_observations (
    observation_id TEXT PRIMARY KEY,
    observation_hash TEXT NOT NULL UNIQUE,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    public_key_id TEXT NOT NULL REFERENCES provider_public_keys(public_key_id),
    signature TEXT NOT NULL,
    canonicalization_version TEXT NOT NULL,
    hardware_fingerprint TEXT NOT NULL,
    host_os TEXT NOT NULL,
    runtime_backend TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_provider_runtime_observations_device_latest
    ON provider_runtime_observations(provider_id, device_id, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_provider_runtime_observations_session_latest
    ON provider_runtime_observations(session_id, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_provider_runtime_observations_key_latest
    ON provider_runtime_observations(public_key_id, server_received_at DESC);

CREATE OR REPLACE FUNCTION prevent_provider_runtime_observation_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'provider_runtime_observations is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS provider_runtime_observations_no_update ON provider_runtime_observations;
CREATE TRIGGER provider_runtime_observations_no_update
    BEFORE UPDATE OR DELETE ON provider_runtime_observations
    FOR EACH ROW EXECUTE FUNCTION prevent_provider_runtime_observation_mutation();
