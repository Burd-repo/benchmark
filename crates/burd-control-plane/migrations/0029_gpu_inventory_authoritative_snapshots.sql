CREATE TABLE IF NOT EXISTS device_gpu_inventory_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    ingest_seq BIGINT GENERATED ALWAYS AS IDENTITY UNIQUE NOT NULL,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    schema_version TEXT NOT NULL,
    inventory_hash TEXT NOT NULL UNIQUE,
    public_key_id TEXT NOT NULL REFERENCES provider_public_keys(public_key_id),
    signature TEXT NOT NULL,
    canonicalization_version TEXT NOT NULL,
    hardware_fingerprint TEXT NOT NULL,
    gpu_count INTEGER NOT NULL CHECK (gpu_count BETWEEN 0 AND 32),
    observed_at TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    verification_json TEXT NOT NULL
);

-- Historical inventory hashes must already represent one internally consistent envelope.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM device_gpu_inventory
        GROUP BY inventory_hash
        HAVING COUNT(*) > 32
            OR COUNT(DISTINCT provider_id) <> 1
            OR COUNT(DISTINCT device_id) <> 1
            OR COUNT(DISTINCT session_id) <> 1
            OR BOOL_OR(session_id IS NULL)
            OR COUNT(DISTINCT schema_version) <> 1
            OR COUNT(DISTINCT public_key_id) <> 1
            OR COUNT(DISTINCT signature) <> 1
            OR COUNT(DISTINCT canonicalization_version) <> 1
            OR COUNT(DISTINCT observed_at) <> 1
            OR COUNT(DISTINCT server_received_at) <> 1
            OR COUNT(DISTINCT payload_json) <> 1
            OR COUNT(DISTINCT verification_json) <> 1
    ) THEN
        RAISE EXCEPTION 'device_gpu_inventory contains inconsistent historical snapshot rows';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM device_gpu_inventory
        WHERE payload_json::jsonb ->> 'provider_id' IS DISTINCT FROM provider_id
           OR payload_json::jsonb ->> 'device_id' IS DISTINCT FROM device_id
           OR payload_json::jsonb ->> 'session_id' IS DISTINCT FROM session_id
           OR payload_json::jsonb ->> 'schema_version' IS DISTINCT FROM schema_version
           OR NULLIF(payload_json::jsonb ->> 'hardware_fingerprint', '') IS NULL
           OR jsonb_typeof(payload_json::jsonb -> 'gpus') IS DISTINCT FROM 'array'
    ) THEN
        RAISE EXCEPTION 'device_gpu_inventory payload binding is inconsistent';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM device_gpu_inventory
        GROUP BY inventory_hash, payload_json
        HAVING jsonb_array_length(payload_json::jsonb -> 'gpus') <> COUNT(*)
    ) THEN
        RAISE EXCEPTION 'device_gpu_inventory payload GPU count is inconsistent';
    END IF;
END;
$$;

-- Existing rows predate ingest_seq. This ordering is deterministic for backfill only and
-- does not claim to reconstruct concurrent historical ingestion perfectly.
INSERT INTO device_gpu_inventory_snapshots (
    snapshot_id,
    provider_id,
    device_id,
    session_id,
    schema_version,
    inventory_hash,
    public_key_id,
    signature,
    canonicalization_version,
    hardware_fingerprint,
    gpu_count,
    observed_at,
    server_received_at,
    payload_json,
    verification_json
)
SELECT
    'gpu_snapshot_backfill_' || inventory_hash,
    MIN(provider_id),
    MIN(device_id),
    MIN(session_id),
    MIN(schema_version),
    inventory_hash,
    MIN(public_key_id),
    MIN(signature),
    MIN(canonicalization_version),
    MIN(payload_json::jsonb ->> 'hardware_fingerprint'),
    COUNT(*)::INTEGER,
    MIN(observed_at),
    MIN(server_received_at),
    MIN(payload_json),
    MIN(verification_json)
FROM device_gpu_inventory
GROUP BY inventory_hash
ORDER BY MIN(server_received_at) ASC, MIN(observed_at) ASC, inventory_hash ASC;

ALTER TABLE device_gpu_inventory
    ADD COLUMN IF NOT EXISTS snapshot_id TEXT;

-- The migration runner wraps this file in one transaction. The append-only trigger is absent
-- only while historical rows receive their snapshot binding.
DROP TRIGGER IF EXISTS device_gpu_inventory_no_update ON device_gpu_inventory;

UPDATE device_gpu_inventory AS inventory
SET snapshot_id = snapshot.snapshot_id
FROM device_gpu_inventory_snapshots AS snapshot
WHERE snapshot.inventory_hash = inventory.inventory_hash
  AND inventory.snapshot_id IS NULL;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM device_gpu_inventory AS inventory
        LEFT JOIN device_gpu_inventory_snapshots AS snapshot
            ON snapshot.snapshot_id = inventory.snapshot_id
        WHERE inventory.snapshot_id IS NULL
           OR snapshot.snapshot_id IS NULL
           OR snapshot.provider_id IS DISTINCT FROM inventory.provider_id
           OR snapshot.device_id IS DISTINCT FROM inventory.device_id
           OR snapshot.session_id IS DISTINCT FROM inventory.session_id
           OR snapshot.inventory_hash IS DISTINCT FROM inventory.inventory_hash
           OR snapshot.public_key_id IS DISTINCT FROM inventory.public_key_id
    ) THEN
        RAISE EXCEPTION 'device_gpu_inventory snapshot backfill is incomplete or inconsistent';
    END IF;
END;
$$;

ALTER TABLE device_gpu_inventory
    ALTER COLUMN session_id SET NOT NULL,
    ALTER COLUMN snapshot_id SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_device_gpu_inventory_snapshot_binding
    ON device_gpu_inventory_snapshots(
        snapshot_id,
        provider_id,
        device_id,
        session_id,
        inventory_hash,
        public_key_id
    );

ALTER TABLE device_gpu_inventory
    ADD CONSTRAINT device_gpu_inventory_snapshot_binding_fk
    FOREIGN KEY (
        snapshot_id,
        provider_id,
        device_id,
        session_id,
        inventory_hash,
        public_key_id
    ) REFERENCES device_gpu_inventory_snapshots(
        snapshot_id,
        provider_id,
        device_id,
        session_id,
        inventory_hash,
        public_key_id
    );

CREATE UNIQUE INDEX IF NOT EXISTS idx_device_gpu_inventory_snapshot_index
    ON device_gpu_inventory(snapshot_id, gpu_index);
CREATE INDEX IF NOT EXISTS idx_device_gpu_inventory_snapshots_device_latest
    ON device_gpu_inventory_snapshots(provider_id, device_id, ingest_seq DESC);
CREATE INDEX IF NOT EXISTS idx_device_gpu_inventory_snapshots_session_latest
    ON device_gpu_inventory_snapshots(session_id, ingest_seq DESC);
CREATE INDEX IF NOT EXISTS idx_device_gpu_inventory_snapshots_key_latest
    ON device_gpu_inventory_snapshots(public_key_id, ingest_seq DESC);

CREATE OR REPLACE FUNCTION prevent_device_gpu_inventory_snapshot_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'device_gpu_inventory_snapshots is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS device_gpu_inventory_snapshots_no_update
    ON device_gpu_inventory_snapshots;
CREATE TRIGGER device_gpu_inventory_snapshots_no_update
    BEFORE UPDATE OR DELETE ON device_gpu_inventory_snapshots
    FOR EACH ROW EXECUTE FUNCTION prevent_device_gpu_inventory_snapshot_mutation();

DROP TRIGGER IF EXISTS device_gpu_inventory_no_update ON device_gpu_inventory;
CREATE TRIGGER device_gpu_inventory_no_update
    BEFORE UPDATE OR DELETE ON device_gpu_inventory
    FOR EACH ROW EXECUTE FUNCTION prevent_device_gpu_inventory_mutation();
