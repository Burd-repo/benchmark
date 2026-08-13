use serde::{Deserialize, Serialize};

pub const USAGE_LEDGER_SCHEMA_VERSION: &str = "burd-usage-ledger-v1";
pub const JOB_USAGE_RECEIPT_SCHEMA_VERSION: &str = "burd-job-usage-receipt-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobUsageReceipt {
    pub schema_version: String,
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_snapshot_id: Option<String>,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub workload_type: String,
    pub gpu_uuid: String,
    pub job_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_completed_at: Option<String>,
    pub reserved_gpu_seconds: u64,
    pub actual_gpu_seconds: u64,
    pub billable_gpu_seconds: u64,
    pub non_billable_gpu_seconds: u64,
    pub idle_billable_gpu_seconds: u64,
    pub idle_unbillable_gpu_seconds: u64,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub network_transfer_bytes: u64,
    pub storage_bytes: u64,
    pub retry_count: u32,
    pub provider_caused_failure: bool,
    pub customer_caused_failure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_classification: Option<String>,
    pub challenge_non_billable_seconds: u64,
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageLedgerEntry {
    pub entry_id: String,
    pub schema_version: String,
    pub entry_type: String,
    pub receipt: JobUsageReceipt,
    pub receipt_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_public_key: Option<String>,
    pub receipt_signature_status: String,
    pub source_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageLedgerResponse {
    pub request_id: String,
    pub entry: UsageLedgerEntry,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListUsageLedgerResponse {
    pub request_id: String,
    pub entries: Vec<UsageLedgerEntry>,
}
