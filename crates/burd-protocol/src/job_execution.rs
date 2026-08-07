use crate::{
    JOB_DATA_PLANE_GRANT_VERSION, JOB_LEASE_SCHEMA_VERSION, JOB_SCHEMA_VERSION, JobDataPlaneGrant,
    JobDataPlaneUrl, JobLeaseRecord, JobRecord, NextJobResponse, ProviderRuntimeCapability,
    validate_provider_runtime_capability,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const PROVIDER_JOB_EXECUTION_SCHEMA_VERSION: &str = "burd-provider-job-execution-v2";
pub const PROVIDER_JOB_EXECUTION_POLICY_VERSION: &str = "burd-provider-job-runtime-policy-v2";
pub const PROVIDER_JOB_APPROVED_TEMPLATES: &[&str] = &[
    "llm_inference",
    "embeddings",
    "image_generation",
    "whisper_transcription",
    "file_processing",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderJobExecutionState {
    Assigned,
    Accepted,
    Provisioning,
    Running,
    Uploading,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
}

impl ProviderJobExecutionState {
    pub fn can_transition_to(self, next: Self) -> bool {
        use ProviderJobExecutionState::*;
        matches!(
            (self, next),
            (Assigned, Accepted | Failed | Cancelled | Expired)
                | (Accepted, Provisioning | Failed | Cancelled | Expired)
                | (Provisioning, Running | Failed | Cancelled)
                | (Running, Uploading | Succeeded | Failed | Cancelled)
                | (Uploading, Succeeded | Failed | Cancelled)
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Expired
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderJobRuntimePolicy {
    pub runtime_engine: String,
    pub container_os: String,
    pub gpu_backend: String,
    pub gpu_runtime: String,
    pub command_source: String,
    pub command_override_allowed: bool,
    pub entrypoint_override_allowed: bool,
    pub network_mode: String,
    pub read_only_rootfs: bool,
    pub no_new_privileges: bool,
    pub run_as_user: String,
    pub seccomp_profile: String,
    pub cap_drop: Vec<String>,
    pub cpu_millis: u32,
    pub memory_mib: u64,
    pub pids_limit: u32,
    pub shm_size_mib: u64,
}

impl ProviderJobRuntimePolicy {
    pub fn v2() -> Self {
        Self {
            runtime_engine: "docker".to_string(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            command_source: "approved_template".to_string(),
            command_override_allowed: false,
            entrypoint_override_allowed: false,
            network_mode: "none".to_string(),
            read_only_rootfs: true,
            no_new_privileges: true,
            run_as_user: "1000:1000".to_string(),
            seccomp_profile: "default".to_string(),
            cap_drop: vec!["ALL".to_string()],
            cpu_millis: 4_000,
            memory_mib: 8_192,
            pids_limit: 512,
            shm_size_mib: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderJobCancellationPolicy {
    pub poll_interval_seconds: u32,
    pub graceful_stop_seconds: u32,
    pub force_kill_after_seconds: u32,
}

impl ProviderJobCancellationPolicy {
    pub fn v1() -> Self {
        Self {
            poll_interval_seconds: 2,
            graceful_stop_seconds: 10,
            force_kill_after_seconds: 15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderJobCleanupPolicy {
    pub remove_container: bool,
    pub remove_working_directory: bool,
    pub clear_ephemeral_secrets: bool,
    pub revoke_data_plane_credential: bool,
}

impl ProviderJobCleanupPolicy {
    pub fn v1() -> Self {
        Self {
            remove_container: true,
            remove_working_directory: true,
            clear_ephemeral_secrets: true,
            revoke_data_plane_credential: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderJobExecutionSpec {
    pub schema_version: String,
    pub policy_version: String,
    pub job_schema_version: String,
    pub lease_schema_version: String,
    pub data_plane_schema_version: String,
    pub job_id: String,
    pub lease_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub workload_type: String,
    pub template_id: String,
    pub image_ref: String,
    pub gpu_uuid: String,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_policy_version: Option<String>,
    pub initial_state: ProviderJobExecutionState,
    pub timeout_seconds: u32,
    pub lease_expires_at: String,
    pub data_plane_credential_expires_at: String,
    pub runtime: ProviderJobRuntimePolicy,
    pub cancellation: ProviderJobCancellationPolicy,
    pub cleanup: ProviderJobCleanupPolicy,
}

pub fn validate_next_job_execution_response(response: &NextJobResponse) -> Result<(), String> {
    match (
        response.job.as_ref(),
        response.data_plane.as_ref(),
        response.lease.as_ref(),
        response.execution.as_ref(),
    ) {
        (None, None, None, None) => Ok(()),
        (Some(job), Some(grant), Some(lease), Some(spec)) => {
            validate_provider_job_execution_bundle(job, lease, grant, spec)
        }
        _ => Err(
            "next-job response must contain either no assignment or a complete execution bundle"
                .to_string(),
        ),
    }
}

pub fn validate_provider_job_execution_bundle(
    job: &JobRecord,
    lease: &JobLeaseRecord,
    grant: &JobDataPlaneGrant,
    spec: &ProviderJobExecutionSpec,
) -> Result<(), String> {
    validate_schema_versions(job, lease, grant, spec)?;
    validate_identity_bindings(job, lease, grant, spec)?;
    validate_workload_bindings(job, lease, spec)?;
    validate_assignment(job, lease, grant, spec)?;
    validate_provider_job_runtime_policy(&spec.runtime)?;
    validate_cancellation_policy(&spec.cancellation)?;
    validate_cleanup_policy(&spec.cleanup)?;
    validate_data_plane_urls(job, grant)?;
    Ok(())
}

fn validate_schema_versions(
    job: &JobRecord,
    lease: &JobLeaseRecord,
    grant: &JobDataPlaneGrant,
    spec: &ProviderJobExecutionSpec,
) -> Result<(), String> {
    if spec.schema_version != PROVIDER_JOB_EXECUTION_SCHEMA_VERSION
        || spec.policy_version != PROVIDER_JOB_EXECUTION_POLICY_VERSION
        || job.schema_version != JOB_SCHEMA_VERSION
        || lease.schema_version != JOB_LEASE_SCHEMA_VERSION
        || grant.schema_version != JOB_DATA_PLANE_GRANT_VERSION
        || spec.job_schema_version != job.schema_version
        || spec.lease_schema_version != lease.schema_version
        || spec.data_plane_schema_version != grant.schema_version
    {
        return Err("execution bundle schema version mismatch".to_string());
    }
    Ok(())
}

fn validate_identity_bindings(
    job: &JobRecord,
    lease: &JobLeaseRecord,
    grant: &JobDataPlaneGrant,
    spec: &ProviderJobExecutionSpec,
) -> Result<(), String> {
    if job.job_id != lease.job_id
        || job.job_id != grant.job_id
        || job.job_id != spec.job_id
        || lease.lease_id != spec.lease_id
        || job.provider_id != lease.provider_id
        || job.provider_id != spec.provider_id
        || job.device_id != lease.device_id
        || job.device_id != spec.device_id
        || job.session_id != lease.session_id
        || job.session_id != spec.session_id
    {
        return Err("execution bundle identity mismatch".to_string());
    }
    Ok(())
}

fn validate_workload_bindings(
    job: &JobRecord,
    lease: &JobLeaseRecord,
    spec: &ProviderJobExecutionSpec,
) -> Result<(), String> {
    if job.workload_type != lease.workload_type
        || job.workload_type != spec.workload_type
        || job.template_id != spec.template_id
        || job.image_ref != spec.image_ref
        || job.gpu_uuid != lease.gpu_uuid
        || job.gpu_uuid != spec.gpu_uuid
        || job.backend != spec.backend
        || job.policy_id != lease.policy_id
        || job.policy_id != spec.policy_id
        || job.policy_version != lease.policy_version
        || job.policy_version != spec.workload_policy_version
    {
        return Err("execution bundle workload mismatch".to_string());
    }
    if !PROVIDER_JOB_APPROVED_TEMPLATES.contains(&job.template_id.as_str()) {
        return Err("execution template is not approved".to_string());
    }
    if job.backend != "cuda" {
        return Err("execution backend must be cuda".to_string());
    }
    validate_digest_pinned_image(&job.image_ref)
}

fn validate_assignment(
    job: &JobRecord,
    lease: &JobLeaseRecord,
    grant: &JobDataPlaneGrant,
    spec: &ProviderJobExecutionSpec,
) -> Result<(), String> {
    if job.status != "assigned"
        || lease.status != "offered"
        || spec.initial_state != ProviderJobExecutionState::Assigned
    {
        return Err("execution bundle is not an offered assignment".to_string());
    }
    if job.timeout_seconds == 0 || job.timeout_seconds != spec.timeout_seconds {
        return Err("execution timeout mismatch".to_string());
    }
    if lease.expires_at != spec.lease_expires_at
        || grant.credential_expires_at != spec.data_plane_credential_expires_at
    {
        return Err("execution expiry mismatch".to_string());
    }
    if grant.credential.is_empty()
        || grant.credential.len() > 512
        || !grant.credential.is_ascii()
        || grant.credential.chars().any(char::is_whitespace)
    {
        return Err("data-plane credential is invalid".to_string());
    }
    Ok(())
}

pub fn validate_provider_job_runtime_policy(
    policy: &ProviderJobRuntimePolicy,
) -> Result<(), String> {
    if policy.runtime_engine != "docker"
        || policy.container_os != "linux"
        || policy.gpu_backend != "cuda"
        || policy.gpu_runtime != "nvidia"
        || policy.command_source != "approved_template"
        || policy.command_override_allowed
        || policy.entrypoint_override_allowed
        || policy.network_mode != "none"
        || !policy.read_only_rootfs
        || !policy.no_new_privileges
        || policy.run_as_user != "1000:1000"
        || policy.seccomp_profile != "default"
        || policy.cap_drop != ["ALL"]
        || policy.cpu_millis == 0
        || policy.memory_mib == 0
        || policy.pids_limit == 0
        || policy.shm_size_mib == 0
    {
        return Err("provider runtime policy is unsafe".to_string());
    }
    Ok(())
}

/// Checks local requirement/capability compatibility without granting backend authority.
///
/// Scheduler admission must additionally require a persisted Control Plane verification record.
pub fn validate_provider_runtime_compatibility(
    policy: &ProviderJobRuntimePolicy,
    capability: &ProviderRuntimeCapability,
) -> Result<(), String> {
    validate_provider_job_runtime_policy(policy)?;
    validate_provider_runtime_capability(capability)?;
    if capability.status != "ready" {
        return Err("provider runtime capability is not ready".to_string());
    }
    if policy.container_os != capability.container_os
        || policy.gpu_backend != capability.gpu_backend
        || policy.gpu_runtime != capability.gpu_runtime
        || !matches!(
            capability.runtime_backend.as_deref(),
            Some("docker_linux_native" | "docker_wsl2")
        )
    {
        return Err("provider runtime capability does not satisfy job policy".to_string());
    }
    Ok(())
}

fn validate_cancellation_policy(policy: &ProviderJobCancellationPolicy) -> Result<(), String> {
    if policy.poll_interval_seconds == 0
        || policy.graceful_stop_seconds == 0
        || policy.force_kill_after_seconds < policy.graceful_stop_seconds
    {
        return Err("provider cancellation policy is invalid".to_string());
    }
    Ok(())
}

fn validate_cleanup_policy(policy: &ProviderJobCleanupPolicy) -> Result<(), String> {
    if !policy.remove_container
        || !policy.remove_working_directory
        || !policy.clear_ephemeral_secrets
        || !policy.revoke_data_plane_credential
    {
        return Err("provider cleanup policy is incomplete".to_string());
    }
    Ok(())
}

fn validate_data_plane_urls(job: &JobRecord, grant: &JobDataPlaneGrant) -> Result<(), String> {
    let input_ids = job
        .input_artifacts
        .iter()
        .map(|artifact| artifact.artifact_id.as_str())
        .collect::<HashSet<_>>();
    let output_ids = job
        .expected_outputs
        .iter()
        .map(|artifact| artifact.artifact_id.as_str())
        .collect::<HashSet<_>>();

    if grant.download_urls.len() != input_ids.len() || grant.upload_urls.len() != output_ids.len() {
        return Err("data-plane artifact count mismatch".to_string());
    }
    for url in &grant.download_urls {
        validate_data_plane_url(job, grant, url, "GET", "artifacts", "download", &input_ids)?;
    }
    for url in &grant.upload_urls {
        validate_data_plane_url(job, grant, url, "PUT", "results", "upload", &output_ids)?;
    }
    Ok(())
}

fn validate_data_plane_url(
    job: &JobRecord,
    grant: &JobDataPlaneGrant,
    url: &JobDataPlaneUrl,
    method: &str,
    collection: &str,
    action: &str,
    expected_ids: &HashSet<&str>,
) -> Result<(), String> {
    let expected_path = format!(
        "/v1/jobs/{}/{}/{}/{}",
        job.job_id, collection, url.artifact_id, action
    );
    if url.method != method
        || !expected_ids.contains(url.artifact_id.as_str())
        || url.url != expected_path
        || url.expires_at != grant.credential_expires_at
        || url.url.contains(&grant.credential)
    {
        return Err("data-plane URL is not bound to the job".to_string());
    }
    Ok(())
}

fn validate_digest_pinned_image(image_ref: &str) -> Result<(), String> {
    let Some((repository, digest)) = image_ref.rsplit_once("@sha256:") else {
        return Err("execution image must be digest-pinned".to_string());
    };
    if repository.is_empty()
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("execution image digest is invalid".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{JobArtifact, PROVIDER_RUNTIME_CAPABILITY_SCHEMA_VERSION};

    struct FakeExecutor {
        state: ProviderJobExecutionState,
    }

    impl FakeExecutor {
        fn transition(&mut self, next: ProviderJobExecutionState) -> Result<(), String> {
            if !self.state.can_transition_to(next) {
                return Err("invalid execution state transition".to_string());
            }
            self.state = next;
            Ok(())
        }
    }

    fn bundle() -> (
        JobRecord,
        JobLeaseRecord,
        JobDataPlaneGrant,
        ProviderJobExecutionSpec,
    ) {
        let input = JobArtifact {
            artifact_id: "prompt".to_string(),
            role: "input".to_string(),
            object_key: "jobs/job_1/prompt.json".to_string(),
            sha256: None,
            size_bytes: None,
            content_type: Some("application/json".to_string()),
        };
        let output = JobArtifact {
            artifact_id: "response".to_string(),
            role: "output".to_string(),
            object_key: "jobs/job_1/response.json".to_string(),
            sha256: None,
            size_bytes: None,
            content_type: Some("application/json".to_string()),
        };
        let job = JobRecord {
            job_id: "job_1".to_string(),
            client_job_id: None,
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            schema_version: JOB_SCHEMA_VERSION.to_string(),
            workload_type: "llm_realtime_api".to_string(),
            template_id: "llm_inference".to_string(),
            image_ref: format!("ghcr.io/burd/llm@sha256:{}", "a".repeat(64)),
            gpu_uuid: "GPU-test".to_string(),
            backend: "cuda".to_string(),
            parameters: serde_json::json!({}),
            input_artifacts: vec![input],
            expected_outputs: vec![output],
            result_artifacts: Vec::new(),
            policy_id: Some("policy_1".to_string()),
            policy_version: Some("v1".to_string()),
            status: "assigned".to_string(),
            progress_percent: None,
            status_message: None,
            error_code: None,
            error_message: None,
            cancellation_reason: None,
            timeout_seconds: 900,
            created_at: "2026-07-29T00:00:00Z".to_string(),
            assigned_at: Some("2026-07-29T00:00:00Z".to_string()),
            accepted_at: None,
            started_at: None,
            completed_at: None,
            updated_at: "2026-07-29T00:00:00Z".to_string(),
        };
        let lease = JobLeaseRecord {
            lease_id: "lease_1".to_string(),
            job_id: job.job_id.clone(),
            provider_id: job.provider_id.clone(),
            device_id: job.device_id.clone(),
            session_id: job.session_id.clone(),
            schema_version: JOB_LEASE_SCHEMA_VERSION.to_string(),
            workload_type: job.workload_type.clone(),
            gpu_uuid: job.gpu_uuid.clone(),
            policy_id: job.policy_id.clone(),
            policy_version: job.policy_version.clone(),
            status: "offered".to_string(),
            reason_codes: Vec::new(),
            offered_at: "2026-07-29T00:00:00Z".to_string(),
            expires_at: "2026-07-29T00:05:00Z".to_string(),
            accepted_at: None,
            provisioning_at: None,
            active_at: None,
            completed_at: None,
            failure_reason: None,
            created_at: "2026-07-29T00:00:00Z".to_string(),
            updated_at: "2026-07-29T00:00:00Z".to_string(),
        };
        let grant = JobDataPlaneGrant {
            schema_version: JOB_DATA_PLANE_GRANT_VERSION.to_string(),
            job_id: job.job_id.clone(),
            credential: "jobcred_example".to_string(),
            credential_expires_at: "2026-07-29T00:20:00Z".to_string(),
            download_urls: vec![JobDataPlaneUrl {
                artifact_id: "prompt".to_string(),
                method: "GET".to_string(),
                url: "/v1/jobs/job_1/artifacts/prompt/download".to_string(),
                expires_at: "2026-07-29T00:20:00Z".to_string(),
            }],
            upload_urls: vec![JobDataPlaneUrl {
                artifact_id: "response".to_string(),
                method: "PUT".to_string(),
                url: "/v1/jobs/job_1/results/response/upload".to_string(),
                expires_at: "2026-07-29T00:20:00Z".to_string(),
            }],
        };
        let spec = ProviderJobExecutionSpec {
            schema_version: PROVIDER_JOB_EXECUTION_SCHEMA_VERSION.to_string(),
            policy_version: PROVIDER_JOB_EXECUTION_POLICY_VERSION.to_string(),
            job_schema_version: job.schema_version.clone(),
            lease_schema_version: lease.schema_version.clone(),
            data_plane_schema_version: grant.schema_version.clone(),
            job_id: job.job_id.clone(),
            lease_id: lease.lease_id.clone(),
            provider_id: job.provider_id.clone(),
            device_id: job.device_id.clone(),
            session_id: job.session_id.clone(),
            workload_type: job.workload_type.clone(),
            template_id: job.template_id.clone(),
            image_ref: job.image_ref.clone(),
            gpu_uuid: job.gpu_uuid.clone(),
            backend: job.backend.clone(),
            policy_id: job.policy_id.clone(),
            workload_policy_version: job.policy_version.clone(),
            initial_state: ProviderJobExecutionState::Assigned,
            timeout_seconds: job.timeout_seconds,
            lease_expires_at: lease.expires_at.clone(),
            data_plane_credential_expires_at: grant.credential_expires_at.clone(),
            runtime: ProviderJobRuntimePolicy::v2(),
            cancellation: ProviderJobCancellationPolicy::v1(),
            cleanup: ProviderJobCleanupPolicy::v1(),
        };
        (job, lease, grant, spec)
    }

    fn windows_runtime_capability() -> ProviderRuntimeCapability {
        ProviderRuntimeCapability {
            schema_version: PROVIDER_RUNTIME_CAPABILITY_SCHEMA_VERSION.to_string(),
            observed_at: "2026-08-07T00:00:00Z".to_string(),
            host_os: "windows".to_string(),
            runtime_backend: Some("docker_wsl2".to_string()),
            runtime_provider: Some("docker_desktop".to_string()),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            status: "ready".to_string(),
            reason_codes: Vec::new(),
            gpu_uuids: vec!["GPU-test".to_string()],
        }
    }

    #[test]
    fn fake_executor_obeys_execution_state_machine() {
        let mut executor = FakeExecutor {
            state: ProviderJobExecutionState::Assigned,
        };
        for state in [
            ProviderJobExecutionState::Accepted,
            ProviderJobExecutionState::Provisioning,
            ProviderJobExecutionState::Running,
            ProviderJobExecutionState::Uploading,
            ProviderJobExecutionState::Succeeded,
        ] {
            executor.transition(state).unwrap();
        }
        assert!(executor.state.is_terminal());
        assert!(
            executor
                .transition(ProviderJobExecutionState::Running)
                .is_err()
        );
    }

    #[test]
    fn valid_bundle_is_bound_without_serializing_credentials_or_commands() {
        let (job, lease, grant, spec) = bundle();
        validate_provider_job_execution_bundle(&job, &lease, &grant, &spec).unwrap();
        let serialized = serde_json::to_string(&spec).unwrap();
        assert!(!serialized.contains(&grant.credential));
        assert!(!serialized.contains("\"command\""));
        assert!(!serialized.contains("target_os"));
        assert!(serialized.contains("\"container_os\":\"linux\""));
        assert!(serialized.contains("\"gpu_backend\":\"cuda\""));
        assert!(serialized.contains("\"gpu_runtime\":\"nvidia\""));
    }

    #[test]
    fn linux_container_policy_is_compatible_with_a_ready_windows_wsl2_host() {
        let policy = ProviderJobRuntimePolicy::v2();
        let capability = windows_runtime_capability();
        validate_provider_runtime_compatibility(&policy, &capability).unwrap();

        let mut unavailable = capability;
        unavailable.status = "not_ready".to_string();
        unavailable.reason_codes = vec!["runtime_backend_verification_required".to_string()];
        assert!(validate_provider_runtime_compatibility(&policy, &unavailable).is_err());
    }

    #[test]
    fn next_job_response_requires_a_complete_execution_bundle() {
        let (job, lease, grant, spec) = bundle();
        let mut response = NextJobResponse {
            request_id: "request_1".to_string(),
            job: Some(job),
            data_plane: Some(grant),
            lease: Some(lease),
            execution: Some(spec),
        };
        validate_next_job_execution_response(&response).unwrap();

        response.execution = None;
        assert!(validate_next_job_execution_response(&response).is_err());

        let empty = NextJobResponse {
            request_id: "request_2".to_string(),
            job: None,
            data_plane: None,
            lease: None,
            execution: None,
        };
        validate_next_job_execution_response(&empty).unwrap();
    }
    #[test]
    fn validation_rejects_cross_lease_and_unsafe_runtime() {
        let (job, mut lease, grant, spec) = bundle();
        lease.lease_id = "lease_2".to_string();
        assert!(validate_provider_job_execution_bundle(&job, &lease, &grant, &spec).is_err());

        let (job, lease, grant, mut spec) = bundle();
        spec.runtime.command_override_allowed = true;
        assert!(validate_provider_job_execution_bundle(&job, &lease, &grant, &spec).is_err());

        spec.runtime.command_override_allowed = false;
        spec.runtime.run_as_user = "0".to_string();
        assert!(validate_provider_job_execution_bundle(&job, &lease, &grant, &spec).is_err());

        let (job, lease, grant, mut spec) = bundle();
        spec.runtime.container_os = "windows".to_string();
        assert!(validate_provider_job_execution_bundle(&job, &lease, &grant, &spec).is_err());

        let (job, lease, grant, mut spec) = bundle();
        spec.runtime.gpu_runtime = "directml".to_string();
        assert!(validate_provider_job_execution_bundle(&job, &lease, &grant, &spec).is_err());
    }
}
