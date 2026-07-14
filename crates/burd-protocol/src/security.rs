use crate::{canonical_json, hash_canonical};
use serde::{Deserialize, Serialize};

pub const SECURITY_POSTURE_SCHEMA_VERSION: &str = "burd-security-posture-v1";
pub const SECURITY_POSTURE_CANONICALIZATION_VERSION: &str = "burd-json-c14n-v1";
pub const SECURITY_POSTURE_SIGNATURE_DOMAIN: &str = "burd.security-posture.v1";
pub const SECURITY_POLICY_VERSION: &str = "burd-security-policy-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyStoragePosture {
    pub storage_backend: String,
    pub hardware_backed: bool,
    pub private_key_exportable: bool,
    pub encrypted_at_rest: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentReleasePosture {
    pub release_channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_hash: Option<String>,
    pub signature_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer_key_id: Option<String>,
    pub auto_update_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttestationPosture {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_hash: Option<String>,
    pub quote_verified_locally: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactIntegrityPosture {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbom_hash: Option<String>,
    pub dependency_scan_status: String,
    pub vulnerability_scan_status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityHardeningPosture {
    pub secrets_backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_runtime: Option<String>,
    pub rbac_enforced: bool,
    pub admin_approval_required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityPosturePayload {
    pub schema_version: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub agent_version: String,
    pub hardware_fingerprint: String,
    pub observed_at: String,
    pub os: String,
    pub architecture: String,
    pub key_storage: KeyStoragePosture,
    pub release: AgentReleasePosture,
    pub attestation: AttestationPosture,
    pub artifact_integrity: ArtifactIntegrityPosture,
    pub hardening: SecurityHardeningPosture,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedSecurityPosture {
    pub payload: SecurityPosturePayload,
    pub posture_hash: String,
    pub public_key_id: String,
    pub signature: String,
    pub canonicalization_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityPostureVerification {
    pub schema_version: String,
    pub posture_hash_valid: bool,
    pub signature_valid: bool,
    pub session_bound: bool,
    pub fingerprint_bound: bool,
    pub active_key_bound: bool,
    pub release_policy_satisfied: bool,
    pub key_storage_satisfied: bool,
    pub attestation_satisfied: bool,
    pub artifact_integrity_satisfied: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityPostureRecord {
    pub posture_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub schema_version: String,
    pub policy_version: String,
    pub status: String,
    pub posture_hash: String,
    pub public_key_id: String,
    pub agent_version: String,
    pub release_channel: String,
    pub key_storage_backend: String,
    pub key_hardware_backed: bool,
    pub private_key_exportable: bool,
    pub attestation_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_evidence_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbom_hash: Option<String>,
    pub vulnerability_scan_status: String,
    pub dependency_scan_status: String,
    pub hardware_fingerprint: String,
    pub observed_at: String,
    pub server_received_at: String,
    pub verification: SecurityPostureVerification,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitSecurityPostureResponse {
    pub request_id: String,
    pub duplicate: bool,
    pub posture: SecurityPostureRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListProviderSecurityPosturesResponse {
    pub request_id: String,
    pub provider_id: String,
    pub records: Vec<SecurityPostureRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityPolicyStatusResponse {
    pub request_id: String,
    pub policy_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_agent_version: Option<String>,
    pub require_signed_agent_release: bool,
    pub require_hardware_backed_key: bool,
    pub require_remote_attestation: bool,
    pub require_sbom_hash: bool,
    pub accepted_release_channels: Vec<String>,
    pub accepted_attestation_modes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SecurityPostureSignatureClaims<'a> {
    domain: &'static str,
    posture_hash: &'a str,
    provider_id: &'a str,
    device_id: &'a str,
    session_id: &'a str,
    hardware_fingerprint: &'a str,
    public_key_id: &'a str,
}

pub fn security_posture_hash(payload: &SecurityPosturePayload) -> Result<String, String> {
    hash_canonical(payload)
}

pub fn security_posture_signature_message(
    payload: &SecurityPosturePayload,
    posture_hash: &str,
    public_key_id: &str,
) -> Result<String, String> {
    canonical_json(&SecurityPostureSignatureClaims {
        domain: SECURITY_POSTURE_SIGNATURE_DOMAIN,
        posture_hash,
        provider_id: &payload.provider_id,
        device_id: &payload.device_id,
        session_id: &payload.session_id,
        hardware_fingerprint: &payload.hardware_fingerprint,
        public_key_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{generate_keypair, sign_message, verify_message};

    fn payload() -> SecurityPosturePayload {
        SecurityPosturePayload {
            schema_version: SECURITY_POSTURE_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            agent_version: "0.1.0".to_string(),
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            observed_at: "2026-07-14T00:00:00Z".to_string(),
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
            key_storage: KeyStoragePosture {
                storage_backend: "software_file".to_string(),
                hardware_backed: false,
                private_key_exportable: true,
                encrypted_at_rest: false,
            },
            release: AgentReleasePosture {
                release_channel: "dev".to_string(),
                binary_hash: Some("sha256:binary".to_string()),
                signature_verified: false,
                signer_key_id: None,
                auto_update_enabled: false,
            },
            attestation: AttestationPosture {
                mode: "none".to_string(),
                evidence_hash: None,
                quote_verified_locally: false,
            },
            artifact_integrity: ArtifactIntegrityPosture {
                sbom_hash: None,
                dependency_scan_status: "not_run".to_string(),
                vulnerability_scan_status: "not_run".to_string(),
            },
            hardening: SecurityHardeningPosture {
                secrets_backend: "filesystem".to_string(),
                sandbox_runtime: Some("docker".to_string()),
                rbac_enforced: false,
                admin_approval_required: true,
            },
            warnings: vec![],
        }
    }

    #[test]
    fn security_posture_signature_binds_identity_and_fingerprint() {
        let payload = payload();
        let hash = security_posture_hash(&payload).unwrap();
        let message = security_posture_signature_message(&payload, &hash, "key_1").unwrap();
        let keys = generate_keypair().unwrap();
        let signature = sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap();
        assert!(verify_message(&keys.public_key_base64, message.as_bytes(), &signature).unwrap());

        let mut changed = payload;
        changed.hardware_fingerprint = "sha256:changed".to_string();
        let changed_message = security_posture_signature_message(&changed, &hash, "key_1").unwrap();
        assert!(
            !verify_message(
                &keys.public_key_base64,
                changed_message.as_bytes(),
                &signature
            )
            .unwrap()
        );
    }
}
