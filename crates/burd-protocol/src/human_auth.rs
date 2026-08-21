use serde::{Deserialize, Serialize};

pub const HUMAN_AUTH_SCHEMA_VERSION: &str = "burd-human-auth-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanIdentitySummary {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub email_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanOrganizationMembershipSummary {
    pub organization_id: String,
    pub role: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanMeResponse {
    pub request_id: String,
    pub schema_version: String,
    pub user_id: String,
    pub status: String,
    pub identity: HumanIdentitySummary,
    #[serde(default)]
    pub organization_memberships: Vec<HumanOrganizationMembershipSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanLogoutResponse {
    pub request_id: String,
    pub revoked_sessions: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListOrganizationMembersResponse {
    pub request_id: String,
    pub members: Vec<crate::OrganizationMembershipRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddOrganizationMemberRequest {
    pub user_id: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOrganizationMemberRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrganizationMemberResponse {
    pub request_id: String,
    pub member: crate::OrganizationMembershipRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListCustomerApiKeysResponse {
    pub request_id: String,
    pub api_keys: Vec<crate::CustomerApiKeyRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevokeCustomerApiKeyResponse {
    pub request_id: String,
    pub api_key: crate::CustomerApiKeyRecord,
}
