CREATE TABLE IF NOT EXISTS workload_policies (
    policy_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    workload_type TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT,
    requirements_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (policy_id, policy_version)
);

CREATE TABLE IF NOT EXISTS provider_workload_eligibility (
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    workload_type TEXT NOT NULL,
    policy_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    engine_version TEXT NOT NULL,
    status TEXT NOT NULL,
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    trust_score DOUBLE PRECISION,
    risk_score DOUBLE PRECISION,
    reliability_score DOUBLE PRECISION,
    verification_status TEXT,
    remote_network_score DOUBLE PRECISION,
    benchmark_result_id TEXT REFERENCES benchmark_results(result_id),
    benchmark_profile_id TEXT,
    benchmark_profile_version TEXT,
    benchmark_backend TEXT,
    benchmark_completed_at TEXT,
    benchmark_status TEXT,
    session_status TEXT,
    latest_gpu_uuid TEXT,
    vram_total_mib BIGINT,
    hardware_fingerprint TEXT,
    regional_reachability_json TEXT NOT NULL DEFAULT '[]',
    evaluated_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (provider_id, device_id, policy_id, policy_version),
    FOREIGN KEY (policy_id, policy_version)
        REFERENCES workload_policies(policy_id, policy_version)
);

CREATE INDEX IF NOT EXISTS idx_workload_policies_workload_status
    ON workload_policies(workload_type, status, policy_version);
CREATE INDEX IF NOT EXISTS idx_provider_workload_eligibility_provider
    ON provider_workload_eligibility(provider_id, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_provider_workload_eligibility_workload_status
    ON provider_workload_eligibility(workload_type, status, updated_at DESC);