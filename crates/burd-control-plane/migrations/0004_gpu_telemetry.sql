CREATE TABLE IF NOT EXISTS telemetry_batches (
    batch_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    control_sequence BIGINT NOT NULL,
    sample_sequence_start BIGINT NOT NULL,
    sample_sequence_end BIGINT NOT NULL,
    hardware_fingerprint TEXT NOT NULL,
    collector TEXT NOT NULL,
    sample_count INTEGER NOT NULL,
    batch_hash TEXT NOT NULL UNIQUE,
    public_key_id TEXT NOT NULL REFERENCES provider_public_keys(public_key_id),
    signature TEXT NOT NULL,
    canonicalization_version TEXT NOT NULL,
    collected_at_start TEXT NOT NULL,
    collected_at_end TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    verification_json TEXT NOT NULL,
    UNIQUE(session_id, control_sequence)
);

CREATE TABLE IF NOT EXISTS gpu_telemetry_samples (
    sample_id TEXT PRIMARY KEY,
    batch_id TEXT NOT NULL REFERENCES telemetry_batches(batch_id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    sample_sequence BIGINT NOT NULL,
    observed_at TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    gpu_uuid TEXT NOT NULL,
    pci_bus_id TEXT NOT NULL,
    gpu_utilization_percent DOUBLE PRECISION,
    memory_utilization_percent DOUBLE PRECISION,
    vram_used_mib BIGINT,
    vram_total_mib BIGINT NOT NULL,
    temperature_celsius DOUBLE PRECISION,
    power_draw_watts DOUBLE PRECISION,
    sample_json TEXT NOT NULL,
    UNIQUE(session_id, sample_sequence)
);

CREATE INDEX IF NOT EXISTS idx_telemetry_batches_session_received
    ON telemetry_batches(session_id, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_telemetry_batches_device_received
    ON telemetry_batches(device_id, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_gpu_telemetry_samples_gpu_received
    ON gpu_telemetry_samples(gpu_uuid, server_received_at DESC);
