use crate::lease::JobLeaseRecord;
use serde::{Deserialize, Serialize};

pub const JOB_SCHEMA_VERSION: &str = "burd-job-v1";
pub const JOB_EVENT_SCHEMA_VERSION: &str = "burd-job-event-v1";
pub const JOB_RESULT_SCHEMA_VERSION: &str = "burd-job-result-v1";
pub const JOB_DATA_PLANE_GRANT_VERSION: &str = "burd-job-data-plane-grant-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobArtifact {
    pub artifact_id: String,
    pub role: String,
    pub object_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobDataPlaneUrl {
    pub artifact_id: String,
    pub method: String,
    pub url: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobDataPlaneGrant {
    pub schema_version: String,
    pub job_id: String,
    pub credential: String,
    pub credential_expires_at: String,
    #[serde(default)]
    pub download_urls: Vec<JobDataPlaneUrl>,
    #[serde(default)]
    pub upload_urls: Vec<JobDataPlaneUrl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateJobRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_job_id: Option<String>,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub workload_type: String,
    pub template_id: String,
    pub image_ref: String,
    pub gpu_uuid: String,
    pub backend: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub input_artifacts: Vec<JobArtifact>,
    #[serde(default)]
    pub expected_outputs: Vec<JobArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_job_id: Option<String>,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub schema_version: String,
    pub workload_type: String,
    pub template_id: String,
    pub image_ref: String,
    pub gpu_uuid: String,
    pub backend: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub input_artifacts: Vec<JobArtifact>,
    #[serde(default)]
    pub expected_outputs: Vec<JobArtifact>,
    #[serde(default)]
    pub result_artifacts: Vec<JobArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation_reason: Option<String>,
    pub timeout_seconds: u32,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub request_id: String,
    pub job: JobRecord,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextJobResponse {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_plane: Option<JobDataPlaneGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<JobLeaseRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AcceptJobRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobEventRequest {
    pub sequence: u64,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobEventRecord {
    pub event_id: String,
    pub job_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub sequence: u64,
    pub schema_version: String,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub occurred_at: String,
    pub server_received_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobEventResponse {
    pub request_id: String,
    pub event: JobEventRecord,
    pub job: JobRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitJobResultRequest {
    pub status: String,
    #[serde(default)]
    pub result_artifacts: Vec<JobArtifact>,
    #[serde(default)]
    pub metrics: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitJobResultResponse {
    pub request_id: String,
    pub job: JobRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelJobRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobResponse {
    pub request_id: String,
    pub job: JobRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListJobsResponse {
    pub request_id: String,
    pub jobs: Vec<JobRecord>,
}
