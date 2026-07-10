use serde::{Deserialize, Serialize};

pub const NETWORK_PROBE_SCHEMA_VERSION: &str = "burd-network-probe-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitNetworkProbeObservationRequest {
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub probe_id: String,
    pub probe_region: String,
    pub observed_at: String,
    pub sample_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_rtt_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet_loss_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_mbps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_mbps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_throughput_mbps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approximate_region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_consistency: Option<f64>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkProbeObservationRecord {
    pub observation_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub probe_id: String,
    pub probe_region: String,
    pub schema_version: String,
    pub observed_at: String,
    pub server_received_at: String,
    pub sample_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_rtt_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet_loss_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_mbps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_mbps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_throughput_mbps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approximate_region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_consistency: Option<f64>,
    pub remote_network_score: f64,
    pub status: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionalReachability {
    pub probe_region: String,
    pub status: String,
    pub remote_network_score: f64,
    pub sample_count: u32,
    pub observed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approximate_region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_rtt_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet_loss_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderNetworkState {
    pub provider_id: String,
    pub device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_network_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_network_score: Option<f64>,
    #[serde(default)]
    pub regional_reachability: Vec<RegionalReachability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_network_score: Option<f64>,
    pub sample_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_observed_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitNetworkProbeObservationResponse {
    pub request_id: String,
    pub duplicate: bool,
    pub observation: NetworkProbeObservationRecord,
    pub network_state: ProviderNetworkState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListNetworkProbeObservationsResponse {
    pub request_id: String,
    pub observations: Vec<NetworkProbeObservationRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListProviderNetworkStatesResponse {
    pub request_id: String,
    pub states: Vec<ProviderNetworkState>,
}
