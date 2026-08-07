use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const SECURE_RUNTIME_SCHEMA_VERSION: &str = "burd-secure-runtime-v2";
pub const SECURE_RUNTIME_POLICY_VERSION: &str = "burd-secure-runtime-policy-v2";
pub const PROVIDER_RUNTIME_CAPABILITY_SCHEMA_VERSION: &str = "burd-provider-runtime-capability-v1";
pub const PROVIDER_RUNTIME_VERIFICATION_SCHEMA_VERSION: &str =
    "burd-provider-runtime-verification-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecureRuntimeCheck {
    pub id: String,
    pub status: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecureRuntimeImageAllowlistEntry {
    pub image_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
    pub signature_status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecureRuntimeResourceLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_count: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mib: Option<u64>,
    pub pids_limit: u32,
    pub shm_size_mib: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecureRuntimeTmpfsMount {
    pub target: String,
    pub size_mib: u64,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecureRuntimeSecurityProfile {
    pub read_only_rootfs: bool,
    pub run_as_user: String,
    pub no_new_privileges: bool,
    #[serde(default)]
    pub cap_drop: Vec<String>,
    pub seccomp_profile: String,
    pub ipc_mode: String,
    pub network_mode: String,
    #[serde(default)]
    pub tmpfs_mounts: Vec<SecureRuntimeTmpfsMount>,
    pub secrets_mode: String,
    pub cleanup_required: bool,
    pub arbitrary_shell_allowed: bool,
}

/// Agent-observed runtime capability. This is never scheduler-authoritative by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRuntimeCapability {
    pub schema_version: String,
    pub observed_at: String,
    pub host_os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_provider: Option<String>,
    pub container_os: String,
    pub gpu_backend: String,
    pub gpu_runtime: String,
    pub isolation_mode: String,
    pub status: String,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    #[serde(default)]
    pub gpu_uuids: Vec<String>,
}

/// Authority state for a runtime capability observation.
///
/// Agent-generated plans use `authority=agent`, `status=reported`, and an unverified GPU
/// binding. Only a future Control Plane proof flow may produce a verified record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRuntimeVerification {
    pub schema_version: String,
    pub authority: String,
    pub status: String,
    pub gpu_uuid_binding: String,
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecureRuntimePlan {
    pub schema_version: String,
    pub policy_version: String,
    pub generated_at: String,
    pub status: String,
    pub capability: ProviderRuntimeCapability,
    pub verification: ProviderRuntimeVerification,
    pub template_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_uuid: Option<String>,
    #[serde(default)]
    pub image_allowlist: Vec<SecureRuntimeImageAllowlistEntry>,
    pub resources: SecureRuntimeResourceLimits,
    pub security: SecureRuntimeSecurityProfile,
    #[serde(default)]
    pub docker_args: Vec<String>,
    #[serde(default)]
    pub checks: Vec<SecureRuntimeCheck>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

pub fn validate_provider_runtime_capability(
    capability: &ProviderRuntimeCapability,
) -> Result<(), String> {
    let unique_gpu_uuids = capability.gpu_uuids.iter().collect::<HashSet<_>>();
    if capability.schema_version != PROVIDER_RUNTIME_CAPABILITY_SCHEMA_VERSION
        || DateTime::parse_from_rfc3339(&capability.observed_at).is_err()
        || !valid_identifier(&capability.host_os)
        || capability.container_os != "linux"
        || capability.gpu_backend != "cuda"
        || capability.gpu_runtime != "nvidia"
        || capability.isolation_mode != "linux_container"
        || !matches!(
            capability.status.as_str(),
            "ready" | "not_ready" | "unsupported"
        )
        || capability.reason_codes.len() > 16
        || capability
            .reason_codes
            .iter()
            .any(|reason| !valid_identifier(reason))
        || capability.gpu_uuids.len() > 32
        || unique_gpu_uuids.len() != capability.gpu_uuids.len()
        || capability.gpu_uuids.iter().any(|uuid| {
            uuid.is_empty()
                || uuid.len() > 128
                || !uuid.is_ascii()
                || uuid.chars().any(char::is_whitespace)
        })
    {
        return Err("provider runtime capability is invalid".to_string());
    }

    match capability.runtime_backend.as_deref() {
        Some("docker_linux_native") if capability.host_os == "linux" => {}
        Some("docker_wsl2") if capability.host_os == "windows" => {}
        Some(_) => return Err("provider runtime backend does not match host OS".to_string()),
        None if capability.status != "ready" => {}
        None => return Err("ready runtime capability requires a backend".to_string()),
    }

    if capability
        .runtime_provider
        .as_deref()
        .is_some_and(|provider| !valid_identifier(provider))
    {
        return Err("provider runtime provider is invalid".to_string());
    }

    if capability.status == "ready"
        && (!capability.reason_codes.is_empty() || capability.gpu_uuids.is_empty())
    {
        return Err("ready runtime capability is incomplete".to_string());
    }
    if capability.status != "ready" && capability.reason_codes.is_empty() {
        return Err("unavailable runtime capability requires reason codes".to_string());
    }
    Ok(())
}

pub fn validate_provider_runtime_verification(
    verification: &ProviderRuntimeVerification,
) -> Result<(), String> {
    if verification.schema_version != PROVIDER_RUNTIME_VERIFICATION_SCHEMA_VERSION
        || verification.reason_codes.len() > 16
        || verification
            .reason_codes
            .iter()
            .any(|reason| !valid_identifier(reason))
    {
        return Err("provider runtime verification is invalid".to_string());
    }

    let valid_state = matches!(
        (
            verification.authority.as_str(),
            verification.status.as_str(),
            verification.gpu_uuid_binding.as_str(),
        ),
        ("agent", "reported", "unverified")
            | ("control_plane", "verified", "verified")
            | ("control_plane", "rejected", "rejected")
    );
    if !valid_state {
        return Err("provider runtime verification authority is invalid".to_string());
    }
    if verification.status == "verified" && !verification.reason_codes.is_empty() {
        return Err("verified runtime capability cannot contain rejection reasons".to_string());
    }
    if verification.status != "verified" && verification.reason_codes.is_empty() {
        return Err("unverified runtime capability requires reason codes".to_string());
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability() -> ProviderRuntimeCapability {
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
    fn windows_wsl2_capability_is_distinct_from_linux_container() {
        let capability = capability();
        validate_provider_runtime_capability(&capability).unwrap();
        let serialized = serde_json::to_value(&capability).unwrap();
        assert_eq!(serialized["host_os"], "windows");
        assert_eq!(serialized["runtime_backend"], "docker_wsl2");
        assert_eq!(serialized["container_os"], "linux");
        assert!(serialized.get("target_os").is_none());
    }

    #[test]
    fn capability_rejects_backend_host_mismatch() {
        let mut capability = capability();
        capability.runtime_backend = Some("docker_linux_native".to_string());
        assert!(validate_provider_runtime_capability(&capability).is_err());
    }

    #[test]
    fn only_control_plane_can_represent_verified_runtime() {
        let reported = ProviderRuntimeVerification {
            schema_version: PROVIDER_RUNTIME_VERIFICATION_SCHEMA_VERSION.to_string(),
            authority: "agent".to_string(),
            status: "reported".to_string(),
            gpu_uuid_binding: "unverified".to_string(),
            reason_codes: vec!["runtime_proof_required".to_string()],
        };
        validate_provider_runtime_verification(&reported).unwrap();

        let mut invalid = reported;
        invalid.status = "verified".to_string();
        invalid.gpu_uuid_binding = "verified".to_string();
        invalid.reason_codes.clear();
        assert!(validate_provider_runtime_verification(&invalid).is_err());
    }
}
