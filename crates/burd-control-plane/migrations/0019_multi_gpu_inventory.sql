CREATE TABLE IF NOT EXISTS device_gpu_inventory (
    inventory_row_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT REFERENCES provider_sessions(session_id),
    schema_version TEXT NOT NULL,
    inventory_hash TEXT NOT NULL UNIQUE,
    public_key_id TEXT NOT NULL REFERENCES provider_public_keys(public_key_id),
    signature TEXT NOT NULL,
    canonicalization_version TEXT NOT NULL,
    gpu_uuid TEXT NOT NULL,
    gpu_index INTEGER NOT NULL,
    backend TEXT NOT NULL,
    pci_vendor_id TEXT NOT NULL,
    pci_device_id TEXT NOT NULL,
    vram_total_mib BIGINT,
    status TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    verification_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_device_gpu_inventory_provider_device_time
    ON device_gpu_inventory(provider_id, device_id, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_device_gpu_inventory_gpu_time
    ON device_gpu_inventory(provider_id, device_id, gpu_uuid, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_device_gpu_inventory_session
    ON device_gpu_inventory(session_id);

CREATE OR REPLACE FUNCTION prevent_device_gpu_inventory_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'device_gpu_inventory is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS device_gpu_inventory_no_update ON device_gpu_inventory;
CREATE TRIGGER device_gpu_inventory_no_update
    BEFORE UPDATE OR DELETE ON device_gpu_inventory
    FOR EACH ROW EXECUTE FUNCTION prevent_device_gpu_inventory_mutation();