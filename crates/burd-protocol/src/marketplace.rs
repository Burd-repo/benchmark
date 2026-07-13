use crate::{BenchmarkResultMetrics, RegionalReachability};
use serde::{Deserialize, Serialize};

pub const MARKETPLACE_LISTING_SCHEMA_VERSION: &str = "burd-marketplace-listing-v1";
pub const MARKETPLACE_ENGINE_VERSION: &str = "burd-marketplace-engine-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RunMarketplaceListingSweepRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default)]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketplaceListingRecord {
    pub listing_id: String,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_display_name: Option<String>,
    pub device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub schema_version: String,
    pub engine_version: String,
    pub status: String,
    pub current_status: String,
    pub workload_type: String,
    pub policy_id: String,
    pub policy_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_uuid: Option<String>,
    pub gpu_verified: bool,
    pub gpu_verification_source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_total_mib: Option<u64>,
    pub vram_verified: bool,
    pub vram_verification_source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub region_source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reliability_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<String>,
    pub proof_freshness_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_network_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_network_score: Option<f64>,
    #[serde(default)]
    pub regional_reachability: Vec<RegionalReachability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_result_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_profile_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark_metrics: Option<BenchmarkResultMetrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_per_hour_micros: Option<u64>,
    pub price_source: String,
    #[serde(default)]
    pub availability_window: serde_json::Value,
    pub active_lease_count: u32,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    pub source_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunMarketplaceListingSweepResponse {
    pub request_id: String,
    pub evaluated: u32,
    pub published: u32,
    pub updated: u32,
    pub skipped: u32,
    #[serde(default)]
    pub listings: Vec<MarketplaceListingRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListMarketplaceListingsResponse {
    pub request_id: String,
    pub listings: Vec<MarketplaceListingRecord>,
}
