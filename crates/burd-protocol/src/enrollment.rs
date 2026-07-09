use crate::identity::default_state_dir;
use crate::signature::canonical_json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub const ENROLLMENT_PROOF_DOMAIN: &str = "burd.enrollment-proof.v1";
pub const KEY_ROTATION_PROOF_DOMAIN: &str = "burd.key-rotation-proof.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueEnrollmentTokenResponse {
    pub request_id: String,
    pub enrollment_token: String,
    pub expires_at: String,
    pub max_uses: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartEnrollmentRequest {
    pub enrollment_token: String,
    pub public_key: String,
    pub key_algorithm: String,
    pub local_provider_id: Option<String>,
    pub machine_id: String,
    pub registration_payload: Value,
    pub hardware_fingerprint: String,
    pub agent_version: String,
    pub benchmark_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartEnrollmentResponse {
    pub request_id: String,
    pub enrollment_id: String,
    pub provider_id: String,
    pub nonce: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentProofRequest {
    pub nonce: String,
    pub signature: String,
    pub public_key: String,
    pub hardware_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentProofResponse {
    pub request_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub public_key_id: String,
    pub credential: String,
    pub credential_expires_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: String,
    pub provider_id: String,
    pub machine_id: Option<String>,
    pub status: String,
    pub active_public_key_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartKeyRotationRequest {
    pub new_public_key: String,
    pub key_algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartKeyRotationResponse {
    pub request_id: String,
    pub rotation_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub current_public_key_id: String,
    pub nonce: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRotationProofRequest {
    pub nonce: String,
    pub signature: String,
    pub new_public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRotationProofResponse {
    pub request_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub public_key_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCredentialResponse {
    pub request_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub credential: String,
    pub credential_expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRevocationResponse {
    pub request_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub status: String,
    pub revoked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEnrollmentState {
    pub control_plane_url: String,
    pub provider_id: String,
    pub device_id: String,
    pub public_key_id: String,
    pub credential: String,
    pub credential_expires_at: String,
    pub enrolled_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEnrollmentStatus {
    pub state_path: String,
    pub control_plane_url: String,
    pub provider_id: String,
    pub device_id: String,
    pub public_key_id: String,
    pub credential_configured: bool,
    pub credential_expires_at: String,
    pub enrolled_at: String,
}

impl RemoteEnrollmentState {
    pub fn public_status(&self) -> RemoteEnrollmentStatus {
        RemoteEnrollmentStatus {
            state_path: remote_enrollment_path().display().to_string(),
            control_plane_url: self.control_plane_url.clone(),
            provider_id: self.provider_id.clone(),
            device_id: self.device_id.clone(),
            public_key_id: self.public_key_id.clone(),
            credential_configured: !self.credential.is_empty(),
            credential_expires_at: self.credential_expires_at.clone(),
            enrolled_at: self.enrolled_at.clone(),
        }
    }
}

pub fn save_remote_enrollment(
    control_plane_url: &str,
    response: &EnrollmentProofResponse,
) -> Result<RemoteEnrollmentStatus, String> {
    let state = RemoteEnrollmentState {
        control_plane_url: control_plane_url.trim_end_matches('/').to_string(),
        provider_id: response.provider_id.clone(),
        device_id: response.device_id.clone(),
        public_key_id: response.public_key_id.clone(),
        credential: response.credential.clone(),
        credential_expires_at: response.credential_expires_at.clone(),
        enrolled_at: Utc::now().to_rfc3339(),
    };
    write_remote_enrollment(&state)?;
    Ok(state.public_status())
}

pub fn update_remote_credential(
    response: &DeviceCredentialResponse,
) -> Result<RemoteEnrollmentStatus, String> {
    let mut state = load_remote_enrollment()?;
    if state.provider_id != response.provider_id || state.device_id != response.device_id {
        return Err("credential response does not match persisted remote identity".to_string());
    }
    state.credential = response.credential.clone();
    state.credential_expires_at = response.credential_expires_at.clone();
    write_remote_enrollment(&state)?;
    Ok(state.public_status())
}

pub fn load_remote_enrollment() -> Result<RemoteEnrollmentState, String> {
    let path = remote_enrollment_path();
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("remote enrollment not found at {}: {error}", path.display()))?;
    serde_json::from_str(&raw).map_err(|error| {
        format!(
            "invalid remote enrollment JSON at {}: {error}",
            path.display()
        )
    })
}

pub fn show_remote_enrollment() -> Result<RemoteEnrollmentStatus, String> {
    Ok(load_remote_enrollment()?.public_status())
}

pub fn remote_enrollment_path() -> PathBuf {
    default_state_dir().join("remote-enrollment.json")
}

fn write_remote_enrollment(state: &RemoteEnrollmentState) -> Result<(), String> {
    let path = remote_enrollment_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|error| format!("failed to serialize remote enrollment: {error}"))?;
    fs::write(&path, json).map_err(|error| {
        format!(
            "failed to write remote enrollment at {}: {error}",
            path.display()
        )
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentProofClaims {
    pub domain: String,
    pub enrollment_id: String,
    pub provider_id: String,
    pub machine_id: String,
    pub nonce: String,
    pub public_key: String,
    pub hardware_fingerprint: String,
    pub expires_at: String,
}

pub fn enrollment_proof_message(
    enrollment_id: &str,
    provider_id: &str,
    machine_id: &str,
    nonce: &str,
    public_key: &str,
    hardware_fingerprint: &str,
    expires_at: &str,
) -> Result<String, String> {
    canonical_json(&EnrollmentProofClaims {
        domain: ENROLLMENT_PROOF_DOMAIN.to_string(),
        enrollment_id: enrollment_id.to_string(),
        provider_id: provider_id.to_string(),
        machine_id: machine_id.to_string(),
        nonce: nonce.to_string(),
        public_key: public_key.to_string(),
        hardware_fingerprint: hardware_fingerprint.to_string(),
        expires_at: expires_at.to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRotationProofClaims {
    pub domain: String,
    pub rotation_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub nonce: String,
    pub current_public_key_id: String,
    pub new_public_key: String,
    pub expires_at: String,
}

pub fn key_rotation_proof_message(
    rotation_id: &str,
    provider_id: &str,
    device_id: &str,
    nonce: &str,
    current_public_key_id: &str,
    new_public_key: &str,
    expires_at: &str,
) -> Result<String, String> {
    canonical_json(&KeyRotationProofClaims {
        domain: KEY_ROTATION_PROOF_DOMAIN.to_string(),
        rotation_id: rotation_id.to_string(),
        provider_id: provider_id.to_string(),
        device_id: device_id.to_string(),
        nonce: nonce.to_string(),
        current_public_key_id: current_public_key_id.to_string(),
        new_public_key: new_public_key.to_string(),
        expires_at: expires_at.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{generate_keypair, sign_message, verify_message};

    #[test]
    fn remote_status_never_serializes_credential() {
        let state = RemoteEnrollmentState {
            control_plane_url: "https://api.burd.cloud".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            public_key_id: "key_1".to_string(),
            credential: "burd_device_secret".to_string(),
            credential_expires_at: "2026-07-08T12:00:00Z".to_string(),
            enrolled_at: "2026-07-08T11:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&state.public_status()).unwrap();
        assert!(!json.contains("burd_device_secret"));
        assert!(!json.contains("\"credential\""));
        assert!(json.contains("\"credential_configured\":true"));
    }

    #[test]
    fn enrollment_proof_is_canonical_and_signature_verifies() {
        let keys = generate_keypair().unwrap();
        let message = enrollment_proof_message(
            "enrollment_1",
            "provider_1",
            "machine_1",
            "nonce_1",
            &keys.public_key_base64,
            "sha256:fingerprint",
            "2026-07-08T12:00:00Z",
        )
        .unwrap();
        let signature = sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap();

        assert!(message.contains(ENROLLMENT_PROOF_DOMAIN));
        assert!(verify_message(&keys.public_key_base64, message.as_bytes(), &signature).unwrap());
    }
}
