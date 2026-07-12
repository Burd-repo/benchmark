CREATE TABLE IF NOT EXISTS benchmark_profiles (
    profile_id TEXT NOT NULL,
    profile_version TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    workload_type TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT,
    image_digest TEXT NOT NULL,
    model_hash TEXT,
    artifact_hash TEXT,
    required_backend TEXT NOT NULL,
    min_vram_gb DOUBLE PRECISION NOT NULL,
    parameters_json TEXT NOT NULL DEFAULT '{}',
    warmup_seconds INTEGER NOT NULL,
    duration_seconds INTEGER NOT NULL,
    sample_count INTEGER NOT NULL,
    thresholds_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (profile_id, profile_version)
);

CREATE TABLE IF NOT EXISTS benchmark_results (
    result_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT NOT NULL REFERENCES provider_sessions(session_id),
    run_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    profile_version TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    workload_type TEXT NOT NULL,
    backend TEXT NOT NULL,
    hardware_fingerprint TEXT NOT NULL,
    gpu_uuid TEXT NOT NULL,
    image_digest TEXT NOT NULL,
    model_hash TEXT,
    artifact_hash TEXT,
    parameters_json TEXT NOT NULL DEFAULT '{}',
    warmup_seconds INTEGER NOT NULL,
    duration_seconds INTEGER NOT NULL,
    sample_count INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    server_received_at TEXT NOT NULL,
    driver_version TEXT NOT NULL,
    cuda_driver_version TEXT,
    cuda_runtime_version TEXT,
    metrics_json TEXT NOT NULL,
    telemetry_window_hash TEXT,
    result_hash TEXT NOT NULL UNIQUE,
    public_key_id TEXT NOT NULL REFERENCES provider_public_keys(public_key_id),
    signature TEXT NOT NULL,
    canonicalization_version TEXT NOT NULL,
    status TEXT NOT NULL,
    verification_json TEXT NOT NULL,
    warnings_json TEXT NOT NULL DEFAULT '[]',
    FOREIGN KEY (profile_id, profile_version)
        REFERENCES benchmark_profiles(profile_id, profile_version),
    UNIQUE(provider_id, device_id, run_id)
);

CREATE INDEX IF NOT EXISTS idx_benchmark_profiles_workload_status
    ON benchmark_profiles(workload_type, status, profile_version);
CREATE INDEX IF NOT EXISTS idx_benchmark_results_provider_time
    ON benchmark_results(provider_id, completed_at DESC, server_received_at DESC);
CREATE INDEX IF NOT EXISTS idx_benchmark_results_profile
    ON benchmark_results(profile_id, profile_version, status);
CREATE INDEX IF NOT EXISTS idx_benchmark_results_gpu
    ON benchmark_results(gpu_uuid, workload_type, completed_at DESC);
