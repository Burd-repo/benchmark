use serde::{Deserialize, Serialize};

pub const CUSTOMER_WORKLOAD_SCHEMA_VERSION: &str = "burd-customer-workload-v1";
pub const COMPUTE_REQUIREMENTS_SCHEMA_VERSION: &str = "burd-compute-requirements-v1";
pub const PLACEMENT_SCHEMA_VERSION: &str = "burd-placement-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeRequirements {
    pub gpu_count: u32,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_vram_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_trust_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_risk_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_reliability_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_price_per_hour_micros: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCustomerWorkloadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_workload_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_id: Option<String>,
    pub workload_type: String,
    pub requirements: ComputeRequirements,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerWorkloadRecord {
    pub workload_id: String,
    pub organization_id: String,
    pub project_id: String,
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_workload_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_id: Option<String>,
    pub workload_type: String,
    pub requirements: ComputeRequirements,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerWorkloadResponse {
    pub request_id: String,
    pub workload: CustomerWorkloadRecord,
    pub duplicate: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> CreateCustomerWorkloadRequest {
        CreateCustomerWorkloadRequest {
            client_workload_id: Some("customer-request-1".to_string()),
            reservation_id: Some("reservation_1".to_string()),
            workload_type: "llm_realtime_api".to_string(),
            requirements: ComputeRequirements {
                gpu_count: 1,
                backend: "cuda".to_string(),
                minimum_vram_mib: Some(16_384),
                region: Some("br-southeast".to_string()),
                minimum_trust_score: Some(80.0),
                maximum_risk_score: Some(20.0),
                minimum_reliability_score: Some(95.0),
                maximum_price_per_hour_micros: Some(2_000_000),
            },
            parameters: serde_json::json!({"max_tokens": 128}),
            timeout_seconds: Some(900),
        }
    }

    #[test]
    fn customer_workload_request_contains_no_physical_target() {
        let value = serde_json::to_value(request()).unwrap();
        for field in [
            "provider_id",
            "device_id",
            "session_id",
            "gpu_uuid",
            "lease_id",
        ] {
            assert!(
                value.get(field).is_none(),
                "unexpected physical field {field}"
            );
        }
        assert_eq!(value["reservation_id"], "reservation_1");
    }

    #[test]
    fn customer_workload_request_rejects_physical_target_fields() {
        let mut value = serde_json::to_value(request()).unwrap();
        value["provider_id"] = serde_json::json!("provider_untrusted");
        assert!(serde_json::from_value::<CreateCustomerWorkloadRequest>(value).is_err());
    }

    #[test]
    fn compute_requirements_reject_unknown_fields() {
        let mut value = serde_json::to_value(request().requirements).unwrap();
        value["gpu_uuid"] = serde_json::json!("GPU-untrusted");
        assert!(serde_json::from_value::<ComputeRequirements>(value).is_err());
    }
}
