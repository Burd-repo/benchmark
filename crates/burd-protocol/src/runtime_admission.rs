use crate::{
    AGENT_RUNTIME_CONTRACT_VERSION, RUNTIME_VERIFICATION_CANONICALIZATION_VERSION,
    RuntimeVerificationChallenge, RuntimeVerificationEvidence, canonical_json, hash_canonical,
};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const PROVIDER_RUNTIME_OBSERVATION_SCHEMA_VERSION: &str =
    "burd-provider-runtime-observation-v1";
pub const RUNTIME_ADMISSION_FINGERPRINT_VERSION: &str = "burd-runtime-admission-fingerprint-v1";
pub const RUNTIME_ADMISSION_SCHEMA_VERSION: &str = "burd-runtime-admission-v1";
pub const RUNTIME_OBSERVATION_SIGNATURE_DOMAIN: &str = "burd.runtime-observation.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRuntimeObservationPayload {
    pub schema_version: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub hardware_fingerprint: String,
    pub host_os: String,
    pub runtime_backend: String,
    pub container_os: String,
    pub gpu_backend: String,
    pub gpu_runtime: String,
    pub isolation_mode: String,
    pub docker_server_version: String,
    pub nvidia_driver_version: String,
    pub nvidia_runtime: String,
    #[serde(default)]
    pub gpu_uuids: Vec<String>,
    pub agent_runtime_contract_version: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedProviderRuntimeObservation {
    pub payload: ProviderRuntimeObservationPayload,
    pub observation_hash: String,
    pub public_key_id: String,
    pub signature: String,
    pub canonicalization_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitProviderRuntimeObservationResponse {
    pub request_id: String,
    pub observation_hash: String,
    pub duplicate: bool,
    pub server_received_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAdmissionFingerprintClaims {
    pub version: String,
    pub provider_id: String,
    pub device_id: String,
    pub hardware_fingerprint: String,
    pub gpu_uuid: String,
    pub host_os: String,
    pub runtime_backend: String,
    pub container_os: String,
    pub gpu_backend: String,
    pub gpu_runtime: String,
    pub isolation_mode: String,
    pub docker_server_version: String,
    pub nvidia_driver_version: String,
    pub nvidia_runtime: String,
    pub agent_runtime_contract_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAdmissionDecision {
    pub schema_version: String,
    pub provider_id: String,
    pub device_id: String,
    pub gpu_uuid: String,
    pub status: String,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_verification_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_observation_hash: Option<String>,
    pub evaluated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProviderRuntimeAdmissionsResponse {
    pub request_id: String,
    pub provider_id: String,
    pub admissions: Vec<RuntimeAdmissionDecision>,
}

#[derive(Debug, Serialize)]
struct RuntimeObservationSignatureClaims<'a> {
    domain: &'static str,
    observation_hash: &'a str,
    provider_id: &'a str,
    device_id: &'a str,
    session_id: &'a str,
    hardware_fingerprint: &'a str,
    public_key_id: &'a str,
}

pub fn provider_runtime_observation_hash(
    payload: &ProviderRuntimeObservationPayload,
) -> Result<String, String> {
    hash_canonical(payload)
}

pub fn provider_runtime_observation_signature_message(
    payload: &ProviderRuntimeObservationPayload,
    observation_hash: &str,
    public_key_id: &str,
) -> Result<String, String> {
    canonical_json(&RuntimeObservationSignatureClaims {
        domain: RUNTIME_OBSERVATION_SIGNATURE_DOMAIN,
        observation_hash,
        provider_id: &payload.provider_id,
        device_id: &payload.device_id,
        session_id: &payload.session_id,
        hardware_fingerprint: &payload.hardware_fingerprint,
        public_key_id,
    })
}

pub fn runtime_admission_fingerprint(
    claims: &RuntimeAdmissionFingerprintClaims,
) -> Result<String, String> {
    hash_canonical(claims)
}

pub fn runtime_admission_claims_from_verification(
    challenge: &RuntimeVerificationChallenge,
    evidence: &RuntimeVerificationEvidence,
) -> RuntimeAdmissionFingerprintClaims {
    RuntimeAdmissionFingerprintClaims {
        version: RUNTIME_ADMISSION_FINGERPRINT_VERSION.to_string(),
        provider_id: challenge.provider_id.clone(),
        device_id: challenge.device_id.clone(),
        hardware_fingerprint: challenge.hardware_fingerprint.clone(),
        gpu_uuid: challenge.gpu_uuid.clone(),
        host_os: evidence.host_os.clone(),
        runtime_backend: evidence.runtime_backend.clone(),
        container_os: evidence.container_os.clone(),
        gpu_backend: evidence.gpu_backend.clone(),
        gpu_runtime: evidence.gpu_runtime.clone(),
        isolation_mode: evidence.isolation_mode.clone(),
        docker_server_version: evidence.docker_server_version.clone(),
        nvidia_driver_version: evidence.nvidia_driver_version.clone(),
        nvidia_runtime: evidence.nvidia_runtime.clone(),
        agent_runtime_contract_version: challenge.agent_runtime_contract_version.clone(),
    }
}

pub fn runtime_admission_claims_from_observation(
    observation: &ProviderRuntimeObservationPayload,
    gpu_uuid: &str,
) -> Result<RuntimeAdmissionFingerprintClaims, String> {
    validate_provider_runtime_observation_payload(observation)?;
    if !observation
        .gpu_uuids
        .iter()
        .any(|value| value.eq_ignore_ascii_case(gpu_uuid))
        || !safe_gpu_uuid(gpu_uuid)
    {
        return Err("runtime observation does not contain the requested GPU".to_string());
    }
    Ok(RuntimeAdmissionFingerprintClaims {
        version: RUNTIME_ADMISSION_FINGERPRINT_VERSION.to_string(),
        provider_id: observation.provider_id.clone(),
        device_id: observation.device_id.clone(),
        hardware_fingerprint: observation.hardware_fingerprint.clone(),
        gpu_uuid: gpu_uuid.to_string(),
        host_os: observation.host_os.clone(),
        runtime_backend: observation.runtime_backend.clone(),
        container_os: observation.container_os.clone(),
        gpu_backend: observation.gpu_backend.clone(),
        gpu_runtime: observation.gpu_runtime.clone(),
        isolation_mode: observation.isolation_mode.clone(),
        docker_server_version: observation.docker_server_version.clone(),
        nvidia_driver_version: observation.nvidia_driver_version.clone(),
        nvidia_runtime: observation.nvidia_runtime.clone(),
        agent_runtime_contract_version: observation.agent_runtime_contract_version.clone(),
    })
}

pub fn validate_provider_runtime_observation_payload(
    payload: &ProviderRuntimeObservationPayload,
) -> Result<(), String> {
    let unique = payload
        .gpu_uuids
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if payload.schema_version != PROVIDER_RUNTIME_OBSERVATION_SCHEMA_VERSION
        || !safe_id(&payload.provider_id, 128)
        || !safe_id(&payload.device_id, 128)
        || !safe_id(&payload.session_id, 128)
        || !safe_hash(&payload.hardware_fingerprint)
        || payload.container_os != "linux"
        || payload.gpu_backend != "cuda"
        || payload.gpu_runtime != "nvidia"
        || payload.isolation_mode != "linux_container"
        || payload.nvidia_runtime != "nvidia"
        || payload.agent_runtime_contract_version != AGENT_RUNTIME_CONTRACT_VERSION
        || payload.gpu_uuids.is_empty()
        || payload.gpu_uuids.len() > 32
        || unique.len() != payload.gpu_uuids.len()
        || payload.gpu_uuids.iter().any(|value| !safe_gpu_uuid(value))
        || !safe_version(&payload.docker_server_version)
        || !safe_version(&payload.nvidia_driver_version)
        || DateTime::parse_from_rfc3339(&payload.observed_at).is_err()
    {
        return Err("provider runtime observation payload is invalid".to_string());
    }
    validate_platform(&payload.host_os, &payload.runtime_backend)
}

pub fn validate_signed_provider_runtime_observation(
    signed: &SignedProviderRuntimeObservation,
) -> Result<(), String> {
    validate_provider_runtime_observation_payload(&signed.payload)?;
    if signed.canonicalization_version != RUNTIME_VERIFICATION_CANONICALIZATION_VERSION
        || !safe_hash(&signed.observation_hash)
        || !safe_id(&signed.public_key_id, 128)
        || signed.signature.is_empty()
        || signed.signature.len() > 512
    {
        return Err("signed provider runtime observation is invalid".to_string());
    }
    Ok(())
}

pub fn validate_runtime_admission_fingerprint_claims(
    claims: &RuntimeAdmissionFingerprintClaims,
) -> Result<(), String> {
    if claims.version != RUNTIME_ADMISSION_FINGERPRINT_VERSION
        || !safe_id(&claims.provider_id, 128)
        || !safe_id(&claims.device_id, 128)
        || !safe_hash(&claims.hardware_fingerprint)
        || !safe_gpu_uuid(&claims.gpu_uuid)
        || claims.container_os != "linux"
        || claims.gpu_backend != "cuda"
        || claims.gpu_runtime != "nvidia"
        || claims.isolation_mode != "linux_container"
        || claims.nvidia_runtime != "nvidia"
        || claims.agent_runtime_contract_version != AGENT_RUNTIME_CONTRACT_VERSION
        || !safe_version(&claims.docker_server_version)
        || !safe_version(&claims.nvidia_driver_version)
    {
        return Err("runtime admission fingerprint claims are invalid".to_string());
    }
    validate_platform(&claims.host_os, &claims.runtime_backend)
}

pub fn validate_runtime_admission_decision(
    decision: &RuntimeAdmissionDecision,
) -> Result<(), String> {
    if decision.schema_version != RUNTIME_ADMISSION_SCHEMA_VERSION
        || !safe_id(&decision.provider_id, 128)
        || !safe_id(&decision.device_id, 128)
        || !safe_gpu_uuid(&decision.gpu_uuid)
        || !matches!(decision.status.as_str(), "admitted" | "denied")
        || (decision.status == "admitted" && !decision.reason_codes.is_empty())
        || (decision.status == "denied" && decision.reason_codes.is_empty())
        || decision
            .reason_codes
            .iter()
            .any(|value| !safe_id(value, 128))
        || DateTime::parse_from_rfc3339(&decision.evaluated_at).is_err()
    {
        return Err("runtime admission decision is invalid".to_string());
    }
    Ok(())
}

fn validate_platform(host_os: &str, runtime_backend: &str) -> Result<(), String> {
    if matches!(
        (host_os, runtime_backend),
        ("linux", "docker_linux_native") | ("windows", "docker_wsl2")
    ) {
        Ok(())
    } else {
        Err("runtime observation backend does not match host OS".to_string())
    }
}

fn safe_id(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

fn safe_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn safe_gpu_uuid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && !value.chars().any(char::is_whitespace)
}

fn safe_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{generate_keypair, sign_message, verify_message};
    use chrono::Utc;

    fn observation() -> ProviderRuntimeObservationPayload {
        ProviderRuntimeObservationPayload {
            schema_version: PROVIDER_RUNTIME_OBSERVATION_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_2".to_string(),
            hardware_fingerprint: "a".repeat(64),
            host_os: "linux".to_string(),
            runtime_backend: "docker_linux_native".to_string(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            docker_server_version: "28.3.0".to_string(),
            nvidia_driver_version: "580.1".to_string(),
            nvidia_runtime: "nvidia".to_string(),
            gpu_uuids: vec!["GPU-A".to_string(), "GPU-B".to_string()],
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            observed_at: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn signed_observation_binds_current_session_and_runtime() {
        let payload = observation();
        validate_provider_runtime_observation_payload(&payload).unwrap();
        let hash = provider_runtime_observation_hash(&payload).unwrap();
        let key = generate_keypair().unwrap();
        let message =
            provider_runtime_observation_signature_message(&payload, &hash, "key_1").unwrap();
        let signature = sign_message(&key.secret_key_base64, message.as_bytes()).unwrap();
        assert!(verify_message(&key.public_key_base64, message.as_bytes(), &signature).unwrap());

        let mut changed = payload;
        changed.docker_server_version = "29.0.0".to_string();
        let changed_hash = provider_runtime_observation_hash(&changed).unwrap();
        let changed_message =
            provider_runtime_observation_signature_message(&changed, &changed_hash, "key_1")
                .unwrap();
        assert!(
            !verify_message(
                &key.public_key_base64,
                changed_message.as_bytes(),
                &signature
            )
            .unwrap()
        );
    }

    #[test]
    fn admission_fingerprint_changes_on_runtime_drift() {
        let observation = observation();
        let original = runtime_admission_fingerprint(
            &runtime_admission_claims_from_observation(&observation, "GPU-B").unwrap(),
        )
        .unwrap();
        let mut changed = observation;
        changed.nvidia_driver_version = "581.0".to_string();
        let changed = runtime_admission_fingerprint(
            &runtime_admission_claims_from_observation(&changed, "GPU-B").unwrap(),
        )
        .unwrap();
        assert_ne!(original, changed);
    }

    #[test]
    fn observation_rejects_duplicate_or_missing_gpu() {
        let mut payload = observation();
        payload.gpu_uuids = vec!["GPU-A".to_string(), "gpu-a".to_string()];
        assert!(validate_provider_runtime_observation_payload(&payload).is_err());

        let payload = observation();
        assert!(runtime_admission_claims_from_observation(&payload, "GPU-C").is_err());
    }
}
