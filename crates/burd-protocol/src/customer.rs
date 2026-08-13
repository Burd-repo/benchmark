use serde::{Deserialize, Serialize};

pub const CUSTOMER_ORGANIZATION_SCHEMA_VERSION: &str = "burd-customer-organization-v1";
pub const CUSTOMER_PROJECT_SCHEMA_VERSION: &str = "burd-customer-project-v1";
pub const CUSTOMER_API_KEY_SCHEMA_VERSION: &str = "burd-customer-api-key-v1";
pub const MARKETPLACE_RESERVATION_SCHEMA_VERSION: &str = "burd-marketplace-reservation-v2";
pub const CUSTOMER_CREDIT_LEDGER_SCHEMA_VERSION: &str = "burd-customer-credit-ledger-v1";
pub const CUSTOMER_AUDIT_SCHEMA_VERSION: &str = "burd-customer-audit-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CreateCustomerUserRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerUserRecord {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerUserResponse {
    pub request_id: String,
    pub user: CustomerUserRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateOrganizationRequest {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrganizationRecord {
    pub organization_id: String,
    pub schema_version: String,
    pub display_name: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrganizationMembershipRecord {
    pub organization_id: String,
    pub user_id: String,
    pub role: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrganizationResponse {
    pub request_id: String,
    pub organization: OrganizationRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership: Option<OrganizationMembershipRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub project_id: String,
    pub organization_id: String,
    pub schema_version: String,
    pub display_name: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectResponse {
    pub request_id: String,
    pub project: ProjectRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertProjectQuotaRequest {
    pub max_active_reservations: u32,
    pub max_reserved_gpu_seconds: u64,
    pub max_reservation_ttl_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectQuotaRecord {
    pub project_id: String,
    pub max_active_reservations: u32,
    pub max_reserved_gpu_seconds: u64,
    pub max_reservation_ttl_seconds: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectQuotaResponse {
    pub request_id: String,
    pub quota: ProjectQuotaRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CreateCustomerApiKeyRequest {
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerApiKeyRecord {
    pub api_key_id: String,
    pub organization_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub schema_version: String,
    pub key_prefix: String,
    pub status: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateCustomerApiKeyResponse {
    pub request_id: String,
    pub api_key: CustomerApiKeyRecord,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantCustomerCreditsRequest {
    pub amount_credits: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerCreditLedgerEntry {
    pub credit_entry_id: String,
    pub organization_id: String,
    pub project_id: String,
    pub schema_version: String,
    pub entry_type: String,
    pub amount_credits: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_id: Option<String>,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerCreditLedgerResponse {
    pub request_id: String,
    pub entry: CustomerCreditLedgerEntry,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateReservationRequest {
    pub listing_id: String,
    pub duration_seconds: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CancelReservationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketplaceReservationRecord {
    pub reservation_id: String,
    pub organization_id: String,
    pub project_id: String,
    pub listing_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_snapshot_id: Option<String>,
    pub provider_id: String,
    pub device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub schema_version: String,
    pub workload_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_uuid: Option<String>,
    pub status: String,
    pub starts_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_at: Option<String>,
    pub reserved_gpu_seconds: u64,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketplaceReservationResponse {
    pub request_id: String,
    pub reservation: MarketplaceReservationRecord,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListMarketplaceReservationsResponse {
    pub request_id: String,
    pub reservations: Vec<MarketplaceReservationRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerUsageSummary {
    pub project_id: String,
    pub active_reservations: u32,
    pub reserved_gpu_seconds: u64,
    pub total_reservations: u32,
    pub cancelled_reservations: u32,
    pub expired_reservations: u32,
    pub credit_balance: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerUsageResponse {
    pub request_id: String,
    pub usage: CustomerUsageSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerAuditEventRecord {
    pub customer_audit_event_id: String,
    pub organization_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub schema_version: String,
    pub actor_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub summary: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListCustomerAuditEventsResponse {
    pub request_id: String,
    pub events: Vec<CustomerAuditEventRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_payload_omits_optional_fields() {
        let request = CreateReservationRequest {
            listing_id: "listing_1".to_string(),
            duration_seconds: 60,
            starts_at: None,
            workload_type: None,
        };
        let serialized = serde_json::to_value(request).unwrap();
        assert_eq!(serialized["listing_id"], "listing_1");
        assert!(serialized.get("starts_at").is_none());
        assert!(serialized.get("workload_type").is_none());
    }

    #[test]
    fn api_key_response_keeps_secret_token_separate_from_record() {
        let response = CreateCustomerApiKeyResponse {
            request_id: "req_1".to_string(),
            api_key: CustomerApiKeyRecord {
                api_key_id: "cak_1".to_string(),
                organization_id: "org_1".to_string(),
                project_id: Some("project_1".to_string()),
                schema_version: CUSTOMER_API_KEY_SCHEMA_VERSION.to_string(),
                key_prefix: "burd_customer_x".to_string(),
                status: "active".to_string(),
                scopes: vec!["reservations:write".to_string()],
                created_at: "2026-01-01T00:00:00Z".to_string(),
                last_used_at: None,
                expires_at: None,
                revoked_at: None,
            },
            token: "burd_customer_secret".to_string(),
        };
        let serialized = serde_json::to_value(response).unwrap();
        assert_eq!(serialized["token"], "burd_customer_secret");
        assert!(serialized["api_key"].get("key_hash").is_none());
    }
}
