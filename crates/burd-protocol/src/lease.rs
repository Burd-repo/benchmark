use serde::{Deserialize, Serialize};

pub const JOB_LEASE_SCHEMA_VERSION: &str = "burd-job-lease-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobLeaseRecord {
    pub lease_id: String,
    pub job_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub schema_version: String,
    pub workload_type: String,
    pub gpu_uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
    pub status: String,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    pub offered_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisioning_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RunSchedulerRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_ttl_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulerDecisionRecord {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_id: Option<String>,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub gpu_uuid: String,
    pub decision: String,
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSchedulerResponse {
    pub request_id: String,
    pub evaluated: u32,
    pub offered: u32,
    pub expired: u32,
    pub skipped: u32,
    #[serde(default)]
    pub decisions: Vec<SchedulerDecisionRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListJobLeasesResponse {
    pub request_id: String,
    pub leases: Vec<JobLeaseRecord>,
}
