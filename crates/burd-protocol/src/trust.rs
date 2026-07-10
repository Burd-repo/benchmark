use serde::{Deserialize, Serialize};

pub const TRUST_POLICY_VERSION: &str = "burd-trust-policy-v1";
pub const ANTIFRAUD_EVENT_SCHEMA_VERSION: &str = "burd-antifraud-event-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunTrustSweepRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default)]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustSweepUpdatedState {
    pub provider_id: String,
    pub device_id: String,
    pub status: String,
    pub trust_score: f64,
    pub risk_score: f64,
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunTrustSweepResponse {
    pub request_id: String,
    pub evaluated: u32,
    pub updated: Vec<TrustSweepUpdatedState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderTrustStateRecord {
    pub provider_id: String,
    pub device_id: String,
    pub status: String,
    pub policy_version: String,
    pub trust_score: f64,
    pub risk_score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reliability_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_network_score: Option<f64>,
    pub evidence_count: u32,
    pub successful_challenge_count: u32,
    pub failed_challenge_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_gpu_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_fingerprint: Option<String>,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListProviderTrustStatesResponse {
    pub request_id: String,
    pub states: Vec<ProviderTrustStateRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AntifraudEventRecord {
    pub event_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub event_type: String,
    pub severity: String,
    pub status: String,
    pub reason: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub occurrence_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListAntifraudEventsResponse {
    pub request_id: String,
    pub events: Vec<AntifraudEventRecord>,
}
