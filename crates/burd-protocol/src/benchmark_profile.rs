use crate::signature::{canonical_json, hash_canonical};
use serde::{Deserialize, Serialize};

pub const BENCHMARK_PROFILE_SCHEMA_VERSION: &str = "burd-benchmark-profile-v2";
pub const BENCHMARK_RESULT_SCHEMA_VERSION: &str = "burd-benchmark-result-v1";
pub const BENCHMARK_RESULT_CANONICALIZATION_VERSION: &str = "burd-json-c14n-v1";
pub const BENCHMARK_RESULT_SIGNATURE_DOMAIN: &str = "burd.benchmark-result.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BenchmarkProfileThresholds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_tokens_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_sustained_tokens_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_requests_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ttft_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_latency_p95_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_error_rate_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertBenchmarkProfileRequest {
    pub profile_id: String,
    pub profile_version: String,
    pub workload_type: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub image_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_hash: Option<String>,
    pub required_backend: String,
    pub min_vram_gb: f64,
    #[serde(default)]
    pub parameters: serde_json::Value,
    pub warmup_seconds: u32,
    pub duration_seconds: u32,
    pub sample_count: u32,
    #[serde(default)]
    pub thresholds: BenchmarkProfileThresholds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkProfileRecord {
    pub profile_id: String,
    pub profile_version: String,
    pub schema_version: String,
    pub workload_type: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub image_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_hash: Option<String>,
    pub required_backend: String,
    pub min_vram_gb: f64,
    #[serde(default)]
    pub parameters: serde_json::Value,
    pub warmup_seconds: u32,
    pub duration_seconds: u32,
    pub sample_count: u32,
    pub thresholds: BenchmarkProfileThresholds,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertBenchmarkProfileResponse {
    pub request_id: String,
    pub profile: BenchmarkProfileRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListBenchmarkProfilesResponse {
    pub request_id: String,
    pub profiles: Vec<BenchmarkProfileRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BenchmarkResultMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sustained_tokens_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_p50_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_p95_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_p99_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance_per_watt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_joules: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_used_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_pressure_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_utilization_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_utilization_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_watts: Option<f64>,
    #[serde(default)]
    pub thermal_throttling_detected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_rate_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkResultPayload {
    pub schema_version: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub run_id: String,
    pub profile_id: String,
    pub profile_version: String,
    pub workload_type: String,
    pub backend: String,
    pub hardware_fingerprint: String,
    pub gpu_uuid: String,
    pub image_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_hash: Option<String>,
    #[serde(default)]
    pub parameters: serde_json::Value,
    pub warmup_seconds: u32,
    pub duration_seconds: u32,
    pub sample_count: u32,
    pub started_at: String,
    pub completed_at: String,
    pub driver_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_driver_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_runtime_version: Option<String>,
    pub metrics: BenchmarkResultMetrics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_window_hash: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedBenchmarkResult {
    pub payload: BenchmarkResultPayload,
    pub result_hash: String,
    pub public_key_id: String,
    pub signature: String,
    pub canonicalization_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkResultVerification {
    pub schema_version: String,
    pub result_hash_valid: bool,
    pub signature_valid: bool,
    pub session_bound: bool,
    pub profile_bound: bool,
    pub backend_bound: bool,
    pub fingerprint_bound: bool,
    pub image_bound: bool,
    pub model_bound: bool,
    pub artifact_bound: bool,
    pub profile_configuration_bound: bool,
    pub metrics_satisfied: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkResultRecord {
    pub result_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub run_id: String,
    pub profile_id: String,
    pub profile_version: String,
    pub schema_version: String,
    pub workload_type: String,
    pub backend: String,
    pub hardware_fingerprint: String,
    pub gpu_uuid: String,
    pub image_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_hash: Option<String>,
    #[serde(default)]
    pub parameters: serde_json::Value,
    pub warmup_seconds: u32,
    pub duration_seconds: u32,
    pub sample_count: u32,
    pub started_at: String,
    pub completed_at: String,
    pub server_received_at: String,
    pub driver_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_driver_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_runtime_version: Option<String>,
    pub metrics: BenchmarkResultMetrics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_window_hash: Option<String>,
    pub result_hash: String,
    pub public_key_id: String,
    pub status: String,
    pub verification: BenchmarkResultVerification,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitBenchmarkResultResponse {
    pub request_id: String,
    pub duplicate: bool,
    pub result: BenchmarkResultRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListProviderBenchmarkResultsResponse {
    pub request_id: String,
    pub results: Vec<BenchmarkResultRecord>,
}

#[derive(Debug, Clone, Serialize)]
struct BenchmarkResultSignatureClaims<'a> {
    domain: &'static str,
    result_hash: &'a str,
    provider_id: &'a str,
    device_id: &'a str,
    session_id: &'a str,
    run_id: &'a str,
    profile_id: &'a str,
    profile_version: &'a str,
    workload_type: &'a str,
    backend: &'a str,
    hardware_fingerprint: &'a str,
    gpu_uuid: &'a str,
    image_digest: &'a str,
    public_key_id: &'a str,
}

pub fn benchmark_result_hash(payload: &BenchmarkResultPayload) -> Result<String, String> {
    hash_canonical(payload)
}

pub fn benchmark_result_signature_message(
    payload: &BenchmarkResultPayload,
    result_hash: &str,
    public_key_id: &str,
) -> Result<String, String> {
    canonical_json(&BenchmarkResultSignatureClaims {
        domain: BENCHMARK_RESULT_SIGNATURE_DOMAIN,
        result_hash,
        provider_id: &payload.provider_id,
        device_id: &payload.device_id,
        session_id: &payload.session_id,
        run_id: &payload.run_id,
        profile_id: &payload.profile_id,
        profile_version: &payload.profile_version,
        workload_type: &payload.workload_type,
        backend: &payload.backend,
        hardware_fingerprint: &payload.hardware_fingerprint,
        gpu_uuid: &payload.gpu_uuid,
        image_digest: &payload.image_digest,
        public_key_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{generate_keypair, sign_message, verify_message};

    fn payload() -> BenchmarkResultPayload {
        BenchmarkResultPayload {
            schema_version: BENCHMARK_RESULT_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            run_id: "run_1".to_string(),
            profile_id: "llm_realtime_api_small".to_string(),
            profile_version: "2026.07.0".to_string(),
            workload_type: "llm_realtime_api".to_string(),
            backend: "cuda".to_string(),
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            image_digest: "sha256:abc".to_string(),
            model_hash: Some("sha256:model".to_string()),
            artifact_hash: None,
            parameters: serde_json::json!({"tokens": 64}),
            warmup_seconds: 5,
            duration_seconds: 60,
            sample_count: 20,
            started_at: "2026-07-11T00:00:00Z".to_string(),
            completed_at: "2026-07-11T00:01:00Z".to_string(),
            driver_version: "576.80".to_string(),
            cuda_driver_version: Some("12.9".to_string()),
            cuda_runtime_version: Some("12.8".to_string()),
            metrics: BenchmarkResultMetrics {
                tokens_per_second: Some(80.0),
                sustained_tokens_per_second: Some(72.0),
                requests_per_second: Some(2.5),
                concurrency: Some(2),
                ttft_ms: Some(180.0),
                latency_p50_ms: Some(600.0),
                latency_p95_ms: Some(900.0),
                latency_p99_ms: Some(1100.0),
                performance_per_watt: Some(0.23),
                power_watts: Some(310.0),
                vram_used_mib: Some(18_000),
                vram_pressure_percent: Some(73.0),
                ..Default::default()
            },
            telemetry_window_hash: Some("sha256:telemetry".to_string()),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn benchmark_result_signature_binds_payload() {
        let payload = payload();
        let hash = benchmark_result_hash(&payload).unwrap();
        let message = benchmark_result_signature_message(&payload, &hash, "key_1").unwrap();
        let keys = generate_keypair().unwrap();
        let signature = sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap();
        assert!(verify_message(&keys.public_key_base64, message.as_bytes(), &signature).unwrap());

        let mut changed = payload;
        changed.gpu_uuid = "GPU-other".to_string();
        let changed_message = benchmark_result_signature_message(&changed, &hash, "key_1").unwrap();
        assert!(
            !verify_message(
                &keys.public_key_base64,
                changed_message.as_bytes(),
                &signature
            )
            .unwrap()
        );
    }
}
