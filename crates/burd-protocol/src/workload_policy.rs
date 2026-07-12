use serde::{Deserialize, Serialize};

pub const WORKLOAD_POLICY_SCHEMA_VERSION: &str = "burd-workload-policy-v2";
pub const WORKLOAD_ELIGIBILITY_SCHEMA_VERSION: &str = "burd-workload-eligibility-v2";
pub const WORKLOAD_POLICY_ENGINE_VERSION: &str = "burd-workload-policy-engine-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WorkloadPolicyRequirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_vram_gb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_profile_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_max_age_seconds: Option<u32>,
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
    pub min_trust_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_risk_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_reliability_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_remote_network_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_verification_status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_regions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price_per_hour: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_proof_max_age_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertWorkloadPolicyRequest {
    pub policy_id: String,
    pub policy_version: String,
    pub workload_type: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub requirements: WorkloadPolicyRequirements,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadPolicyRecord {
    pub policy_id: String,
    pub policy_version: String,
    pub schema_version: String,
    pub workload_type: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub requirements: WorkloadPolicyRequirements,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertWorkloadPolicyResponse {
    pub request_id: String,
    pub policy: WorkloadPolicyRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListWorkloadPoliciesResponse {
    pub request_id: String,
    pub policies: Vec<WorkloadPolicyRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunWorkloadEligibilityRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default)]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadEligibilityRecord {
    pub provider_id: String,
    pub device_id: String,
    pub workload_type: String,
    pub policy_id: String,
    pub policy_version: String,
    pub schema_version: String,
    pub engine_version: String,
    pub status: String,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reliability_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_network_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_result_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_profile_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_gpu_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_fingerprint: Option<String>,
    pub evaluated_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunWorkloadEligibilityResponse {
    pub request_id: String,
    pub evaluated: u32,
    pub updated: Vec<WorkloadEligibilityRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListProviderWorkloadEligibilityResponse {
    pub request_id: String,
    pub states: Vec<WorkloadEligibilityRecord>,
}
