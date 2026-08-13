use serde::{Deserialize, Serialize};

pub const CUSTOMER_ARTIFACT_SCHEMA_VERSION: &str = "burd-customer-artifact-v1";
pub const CUSTOMER_ARTIFACT_UPLOAD_INTENT_SCHEMA_VERSION: &str =
    "burd-customer-artifact-upload-intent-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCustomerArtifactRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_artifact_id: Option<String>,
    pub sha256: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerArtifactRecord {
    pub artifact_id: String,
    pub organization_id: String,
    pub project_id: String,
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_artifact_id: Option<String>,
    pub status: String,
    pub sha256: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub upload_expires_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploaded_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerArtifactUploadTarget {
    pub method: String,
    pub url: String,
    pub expires_at: String,
    pub content_length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerArtifactUploadIntentResponse {
    pub schema_version: String,
    pub request_id: String,
    pub artifact: CustomerArtifactRecord,
    pub upload: CustomerArtifactUploadTarget,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerArtifactResponse {
    pub request_id: String,
    pub artifact: CustomerArtifactRecord,
    pub duplicate: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customer_artifact_contract_exposes_no_storage_credentials() {
        let response = CustomerArtifactUploadIntentResponse {
            schema_version: CUSTOMER_ARTIFACT_UPLOAD_INTENT_SCHEMA_VERSION.to_string(),
            request_id: "req_1".to_string(),
            artifact: CustomerArtifactRecord {
                artifact_id: "artifact_1".to_string(),
                organization_id: "org_1".to_string(),
                project_id: "project_1".to_string(),
                schema_version: CUSTOMER_ARTIFACT_SCHEMA_VERSION.to_string(),
                client_artifact_id: None,
                status: "pending_upload".to_string(),
                sha256: format!("sha256:{}", "a".repeat(64)),
                size_bytes: 12,
                content_type: Some("application/json".to_string()),
                upload_expires_at: "2026-08-12T12:15:00Z".to_string(),
                expires_at: "2026-08-19T12:00:00Z".to_string(),
                uploaded_at: None,
                ready_at: None,
                created_at: "2026-08-12T12:00:00Z".to_string(),
                updated_at: "2026-08-12T12:00:00Z".to_string(),
            },
            upload: CustomerArtifactUploadTarget {
                method: "PUT".to_string(),
                url: "/v1/customer/projects/project_1/artifacts/artifact_1/content".to_string(),
                expires_at: "2026-08-12T12:15:00Z".to_string(),
                content_length: 12,
                sha256: format!("sha256:{}", "a".repeat(64)),
            },
            duplicate: false,
        };
        let value = serde_json::to_value(response).unwrap();
        let serialized = value.to_string();
        for secret in ["credential", "object_key", "provider_id", "device_id"] {
            assert!(!serialized.contains(secret));
        }
    }
}
