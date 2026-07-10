CREATE TABLE IF NOT EXISTS network_probe_observations (
    observation_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    probe_id TEXT NOT NULL,
    probe_region TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    sample_count INTEGER NOT NULL,
    control_rtt_ms DOUBLE PRECISION,
    jitter_ms DOUBLE PRECISION,
    packet_loss_percent DOUBLE PRECISION,
    reconnect_count INTEGER,
    upload_mbps DOUBLE PRECISION,
    download_mbps DOUBLE PRECISION,
    artifact_throughput_mbps DOUBLE PRECISION,
    stability_score DOUBLE PRECISION,
    approximate_region TEXT,
    path_consistency DOUBLE PRECISION,
    remote_network_score DOUBLE PRECISION NOT NULL,
    status TEXT NOT NULL,
    warnings_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}',
    UNIQUE(session_id, probe_id, observed_at)
);

CREATE TABLE IF NOT EXISTS provider_network_states (
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    local_network_score DOUBLE PRECISION,
    remote_network_score DOUBLE PRECISION,
    regional_reachability_json TEXT NOT NULL DEFAULT '[]',
    effective_network_score DOUBLE PRECISION,
    sample_count INTEGER NOT NULL DEFAULT 0,
    last_observed_at TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (provider_id, device_id)
);

CREATE INDEX IF NOT EXISTS idx_network_probe_observations_provider_time
    ON network_probe_observations(provider_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_network_probe_observations_session_time
    ON network_probe_observations(session_id, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_network_probe_observations_region_time
    ON network_probe_observations(probe_region, observed_at DESC);
CREATE INDEX IF NOT EXISTS idx_provider_network_states_score
    ON provider_network_states(effective_network_score, updated_at);