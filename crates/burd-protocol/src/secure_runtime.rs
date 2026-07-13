use serde::{Deserialize, Serialize};

pub const SECURE_RUNTIME_SCHEMA_VERSION: &str = "burd-secure-runtime-v1";
pub const SECURE_RUNTIME_POLICY_VERSION: &str = "burd-secure-runtime-policy-v1";

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecureRuntimePlan {
    pub schema_version: String,
    pub policy_version: String,
    pub generated_at: String,
    pub status: String,
    pub runtime_engine: String,
    pub target_os: String,
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
