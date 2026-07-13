CREATE TABLE IF NOT EXISTS marketplace_listings (
    listing_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    provider_display_name TEXT,
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT REFERENCES provider_sessions(session_id),
    schema_version TEXT NOT NULL,
    engine_version TEXT NOT NULL,
    status TEXT NOT NULL,
    current_status TEXT NOT NULL,
    workload_type TEXT NOT NULL,
    policy_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    gpu_uuid TEXT,
    gpu_verified BOOLEAN NOT NULL,
    gpu_verification_source TEXT NOT NULL,
    vram_total_mib BIGINT,
    vram_verified BOOLEAN NOT NULL,
    vram_verification_source TEXT NOT NULL,
    region TEXT,
    region_source TEXT NOT NULL,
    trust_score DOUBLE PRECISION,
    risk_score DOUBLE PRECISION,
    reliability_score DOUBLE PRECISION,
    verification_status TEXT,
    proof_freshness_status TEXT NOT NULL,
    last_verified_at TEXT,
    remote_network_score DOUBLE PRECISION,
    effective_network_score DOUBLE PRECISION,
    regional_reachability_json TEXT NOT NULL DEFAULT '[]',
    benchmark_result_id TEXT REFERENCES benchmark_results(result_id),
    benchmark_profile_id TEXT,
    benchmark_profile_version TEXT,
    benchmark_status TEXT,
    benchmark_completed_at TEXT,
    benchmark_metrics_json TEXT,
    price_currency TEXT,
    price_per_hour_micros BIGINT,
    price_source TEXT NOT NULL,
    availability_window_json TEXT NOT NULL DEFAULT '{}',
    active_lease_count INTEGER NOT NULL DEFAULT 0,
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    source_hash TEXT NOT NULL,
    published_at TEXT,
    updated_at TEXT NOT NULL,
    UNIQUE(provider_id, device_id, workload_type, policy_id, policy_version),
    FOREIGN KEY (policy_id, policy_version)
        REFERENCES workload_policies(policy_id, policy_version)
);

CREATE INDEX IF NOT EXISTS idx_marketplace_listings_status_workload
    ON marketplace_listings(status, workload_type, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_marketplace_listings_provider
    ON marketplace_listings(provider_id, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_marketplace_listings_scores
    ON marketplace_listings(workload_type, trust_score DESC, reliability_score DESC, remote_network_score DESC);
CREATE INDEX IF NOT EXISTS idx_marketplace_listings_gpu
    ON marketplace_listings(gpu_uuid, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_marketplace_listings_benchmark
    ON marketplace_listings(benchmark_result_id, benchmark_completed_at DESC);
