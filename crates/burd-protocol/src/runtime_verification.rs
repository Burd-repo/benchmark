use crate::{
    RuntimeAdmissionFingerprintClaims, canonical_json, hash_canonical,
    runtime_admission_fingerprint, validate_runtime_admission_fingerprint_claims,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const RUNTIME_VERIFICATION_CHALLENGE_SCHEMA_VERSION: &str =
    "burd-runtime-verification-challenge-v1";
pub const RUNTIME_VERIFICATION_RESPONSE_SCHEMA_VERSION: &str =
    "burd-runtime-verification-response-v1";
pub const RUNTIME_VERIFICATION_RECORD_SCHEMA_VERSION: &str = "burd-runtime-verification-record-v2";
pub const RUNTIME_VERIFICATION_CANONICALIZATION_VERSION: &str = "burd-canonical-json-v1";
pub const RUNTIME_VERIFICATION_SIGNATURE_DOMAIN: &str = "burd.runtime-verification.v1";
pub const RUNTIME_VERIFICATION_FINGERPRINT_VERSION: &str =
    "burd-runtime-verification-fingerprint-v1";
pub const RUNTIME_PROOF_POLICY_VERSION: &str = "burd-runtime-proof-policy-v1";
pub const AGENT_RUNTIME_CONTRACT_VERSION: &str = "burd-agent-runtime-contract-v1";
pub const RUNTIME_PROOF_OUTPUT_SCHEMA_VERSION: &str = "burd-runtime-proof-output-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRuntimeVerificationChallengeRequest {
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub gpu_uuid: String,
    pub runtime_backend: String,
    pub proof_image_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_ttl_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeVerificationChallenge {
    pub schema_version: String,
    pub challenge_id: String,
    pub nonce: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub hardware_fingerprint: String,
    pub host_os: String,
    pub gpu_uuid: String,
    pub runtime_backend: String,
    pub container_os: String,
    pub gpu_backend: String,
    pub gpu_runtime: String,
    pub isolation_mode: String,
    pub proof_image_ref: String,
    pub proof_policy_version: String,
    pub agent_runtime_contract_version: String,
    pub issued_at: String,
    pub expires_at: String,
    pub verification_ttl_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProofOutput {
    pub schema_version: String,
    pub nonce: String,
    pub observed_gpu_uuids: Vec<String>,
    pub nvidia_driver_version: String,
    pub cuda_runtime_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeVerificationEvidence {
    pub host_os: String,
    pub runtime_backend: String,
    pub container_os: String,
    pub gpu_backend: String,
    pub gpu_runtime: String,
    pub isolation_mode: String,
    pub docker_server_version: String,
    pub nvidia_driver_version: String,
    pub nvidia_runtime: String,
    pub cuda_runtime_version: String,
    pub observed_gpu_uuids: Vec<String>,
    pub proof_image_digest: String,
    pub proof_nonce: String,
    pub network_mode: String,
    pub run_as_user: String,
    pub read_only_rootfs: bool,
    pub no_new_privileges: bool,
    pub cap_drop: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeVerificationFingerprintClaims {
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
    pub cuda_runtime_version: String,
    pub agent_runtime_contract_version: String,
    pub proof_policy_version: String,
    pub proof_image_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeVerificationResponsePayload {
    pub schema_version: String,
    pub challenge_id: String,
    pub nonce: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub hardware_fingerprint: String,
    pub gpu_uuid: String,
    pub runtime_backend: String,
    pub proof_policy_version: String,
    pub agent_runtime_contract_version: String,
    pub runtime_verification_fingerprint: String,
    pub evidence: RuntimeVerificationEvidence,
    pub started_at: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRuntimeVerificationResponse {
    pub payload: RuntimeVerificationResponsePayload,
    pub response_hash: String,
    pub public_key_id: String,
    pub signature: String,
    pub canonicalization_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRuntimeVerificationRecord {
    pub schema_version: String,
    pub verification_id: String,
    pub challenge_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub hardware_fingerprint: String,
    pub gpu_uuid: String,
    pub host_os: String,
    pub runtime_backend: String,
    pub status: String,
    pub gpu_uuid_binding: String,
    pub runtime_verification_fingerprint: String,
    pub proof_policy_version: String,
    pub agent_runtime_contract_version: String,
    pub proof_image_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_admission_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_admission_claims: Option<RuntimeAdmissionFingerprintClaims>,
    pub verified_at: String,
    pub expires_at: String,
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeVerificationChallengeRecord {
    pub challenge: RuntimeVerificationChallenge,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expired_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<ProviderRuntimeVerificationRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRuntimeVerificationChallengeResponse {
    pub request_id: String,
    pub challenge: RuntimeVerificationChallenge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextRuntimeVerificationChallengeResponse {
    pub request_id: String,
    pub challenge: RuntimeVerificationChallenge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitRuntimeVerificationResponse {
    pub request_id: String,
    pub challenge_id: String,
    pub status: String,
    pub response_hash: String,
    pub server_received_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<ProviderRuntimeVerificationRecord>,
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProviderRuntimeVerificationsResponse {
    pub request_id: String,
    pub verifications: Vec<ProviderRuntimeVerificationRecord>,
}

#[derive(Debug, Serialize)]
struct RuntimeVerificationSignatureClaims<'a> {
    domain: &'static str,
    response_hash: &'a str,
    challenge_id: &'a str,
    nonce: &'a str,
    provider_id: &'a str,
    device_id: &'a str,
    session_id: &'a str,
    hardware_fingerprint: &'a str,
    gpu_uuid: &'a str,
    runtime_backend: &'a str,
    runtime_verification_fingerprint: &'a str,
    proof_policy_version: &'a str,
    agent_runtime_contract_version: &'a str,
    public_key_id: &'a str,
}

pub fn runtime_verification_fingerprint(
    claims: &RuntimeVerificationFingerprintClaims,
) -> Result<String, String> {
    hash_canonical(claims)
}

pub fn runtime_verification_response_hash(
    payload: &RuntimeVerificationResponsePayload,
) -> Result<String, String> {
    hash_canonical(payload)
}

pub fn runtime_verification_signature_message(
    payload: &RuntimeVerificationResponsePayload,
    response_hash: &str,
    public_key_id: &str,
) -> Result<String, String> {
    canonical_json(&RuntimeVerificationSignatureClaims {
        domain: RUNTIME_VERIFICATION_SIGNATURE_DOMAIN,
        response_hash,
        challenge_id: &payload.challenge_id,
        nonce: &payload.nonce,
        provider_id: &payload.provider_id,
        device_id: &payload.device_id,
        session_id: &payload.session_id,
        hardware_fingerprint: &payload.hardware_fingerprint,
        gpu_uuid: &payload.gpu_uuid,
        runtime_backend: &payload.runtime_backend,
        runtime_verification_fingerprint: &payload.runtime_verification_fingerprint,
        proof_policy_version: &payload.proof_policy_version,
        agent_runtime_contract_version: &payload.agent_runtime_contract_version,
        public_key_id,
    })
}

pub fn fingerprint_claims(
    challenge: &RuntimeVerificationChallenge,
    evidence: &RuntimeVerificationEvidence,
) -> RuntimeVerificationFingerprintClaims {
    RuntimeVerificationFingerprintClaims {
        version: RUNTIME_VERIFICATION_FINGERPRINT_VERSION.to_string(),
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
        cuda_runtime_version: evidence.cuda_runtime_version.clone(),
        agent_runtime_contract_version: challenge.agent_runtime_contract_version.clone(),
        proof_policy_version: challenge.proof_policy_version.clone(),
        proof_image_digest: evidence.proof_image_digest.clone(),
    }
}

pub fn validate_runtime_verification_challenge(
    challenge: &RuntimeVerificationChallenge,
) -> Result<(), String> {
    let issued = parse_time(&challenge.issued_at)?;
    let expires = parse_time(&challenge.expires_at)?;
    if challenge.schema_version != RUNTIME_VERIFICATION_CHALLENGE_SCHEMA_VERSION
        || !safe_id(&challenge.challenge_id, 128)
        || !safe_nonce(&challenge.nonce)
        || !safe_id(&challenge.provider_id, 128)
        || !safe_id(&challenge.device_id, 128)
        || !safe_id(&challenge.session_id, 128)
        || !safe_hash(&challenge.hardware_fingerprint)
        || !safe_gpu_uuid(&challenge.gpu_uuid)
        || !immutable_image_ref(&challenge.proof_image_ref)
        || challenge.proof_policy_version != RUNTIME_PROOF_POLICY_VERSION
        || challenge.agent_runtime_contract_version != AGENT_RUNTIME_CONTRACT_VERSION
        || challenge.container_os != "linux"
        || challenge.gpu_backend != "cuda"
        || challenge.gpu_runtime != "nvidia"
        || challenge.isolation_mode != "linux_container"
        || challenge.verification_ttl_seconds == 0
        || challenge.verification_ttl_seconds > 604_800
        || expires <= issued
    {
        return Err("runtime verification challenge is invalid".to_string());
    }
    validate_platform(&challenge.host_os, &challenge.runtime_backend)
}

pub fn validate_runtime_verification_evidence(
    challenge: &RuntimeVerificationChallenge,
    evidence: &RuntimeVerificationEvidence,
) -> Result<(), String> {
    let unique = evidence.observed_gpu_uuids.iter().collect::<HashSet<_>>();
    if evidence.host_os != challenge.host_os
        || evidence.runtime_backend != challenge.runtime_backend
        || evidence.container_os != challenge.container_os
        || evidence.gpu_backend != challenge.gpu_backend
        || evidence.gpu_runtime != challenge.gpu_runtime
        || evidence.isolation_mode != challenge.isolation_mode
        || evidence.proof_image_digest != challenge.proof_image_ref
        || evidence.proof_nonce != challenge.nonce
        || evidence.network_mode != "none"
        || evidence.run_as_user != "1000:1000"
        || !evidence.read_only_rootfs
        || !evidence.no_new_privileges
        || evidence.cap_drop != ["ALL"]
        || evidence.nvidia_runtime != "nvidia"
        || evidence.observed_gpu_uuids != [challenge.gpu_uuid.clone()]
        || unique.len() != evidence.observed_gpu_uuids.len()
        || !safe_version(&evidence.docker_server_version)
        || !safe_version(&evidence.nvidia_driver_version)
        || !safe_version(&evidence.cuda_runtime_version)
    {
        return Err("runtime verification evidence is invalid".to_string());
    }
    Ok(())
}

pub fn validate_signed_runtime_verification_response(
    signed: &SignedRuntimeVerificationResponse,
) -> Result<(), String> {
    let payload = &signed.payload;
    if payload.schema_version != RUNTIME_VERIFICATION_RESPONSE_SCHEMA_VERSION
        || signed.canonicalization_version != RUNTIME_VERIFICATION_CANONICALIZATION_VERSION
        || !safe_hash(&signed.response_hash)
        || !safe_hash(&payload.runtime_verification_fingerprint)
        || !safe_id(&signed.public_key_id, 128)
        || signed.signature.is_empty()
        || signed.signature.len() > 512
        || DateTime::parse_from_rfc3339(&payload.started_at).is_err()
        || DateTime::parse_from_rfc3339(&payload.completed_at).is_err()
    {
        return Err("signed runtime verification response is invalid".to_string());
    }
    Ok(())
}

pub fn validate_provider_runtime_verification_record(
    record: &ProviderRuntimeVerificationRecord,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let verified_at = parse_time(&record.verified_at)?;
    let expires_at = parse_time(&record.expires_at)?;
    let public_key_id = record.public_key_id.as_deref().unwrap_or_default();
    let admission_fingerprint = record
        .runtime_admission_fingerprint
        .as_deref()
        .unwrap_or_default();
    let admission_claims_valid = record
        .runtime_admission_claims
        .as_ref()
        .is_some_and(|claims| {
            validate_runtime_admission_fingerprint_claims(claims).is_ok()
                && claims.provider_id == record.provider_id
                && claims.device_id == record.device_id
                && claims.hardware_fingerprint == record.hardware_fingerprint
                && claims.gpu_uuid == record.gpu_uuid
                && claims.host_os == record.host_os
                && claims.runtime_backend == record.runtime_backend
                && claims.agent_runtime_contract_version == record.agent_runtime_contract_version
                && runtime_admission_fingerprint(claims)
                    .is_ok_and(|value| value == admission_fingerprint)
        });
    if record.schema_version != RUNTIME_VERIFICATION_RECORD_SCHEMA_VERSION
        || !safe_id(&record.verification_id, 128)
        || !safe_id(&record.challenge_id, 128)
        || !safe_id(&record.provider_id, 128)
        || !safe_id(&record.device_id, 128)
        || !safe_id(&record.session_id, 128)
        || !safe_hash(&record.hardware_fingerprint)
        || !safe_gpu_uuid(&record.gpu_uuid)
        || !safe_hash(&record.runtime_verification_fingerprint)
        || record.proof_policy_version != RUNTIME_PROOF_POLICY_VERSION
        || record.agent_runtime_contract_version != AGENT_RUNTIME_CONTRACT_VERSION
        || !immutable_image_ref(&record.proof_image_digest)
        || !safe_id(public_key_id, 128)
        || !safe_hash(admission_fingerprint)
        || !admission_claims_valid
        || record.status != "verified"
        || record.gpu_uuid_binding != "verified"
        || !record.reason_codes.is_empty()
        || verified_at > now
        || expires_at <= verified_at
        || expires_at <= now
        || expires_at - verified_at > chrono::Duration::seconds(604_800)
    {
        return Err("provider runtime verification record is invalid or expired".to_string());
    }
    validate_platform(&record.host_os, &record.runtime_backend)
}

pub fn immutable_image_ref(value: &str) -> bool {
    let Some((name, digest)) = value.rsplit_once("@sha256:") else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 512
        && !name.chars().any(char::is_whitespace)
        && digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_platform(host_os: &str, runtime_backend: &str) -> Result<(), String> {
    if matches!(
        (host_os, runtime_backend),
        ("linux", "docker_linux_native") | ("windows", "docker_wsl2")
    ) {
        Ok(())
    } else {
        Err("runtime verification backend does not match host OS".to_string())
    }
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| "runtime verification timestamp is invalid".to_string())
}

fn safe_id(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

fn safe_nonce(value: &str) -> bool {
    safe_id(value, 256)
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

    fn challenge() -> RuntimeVerificationChallenge {
        RuntimeVerificationChallenge {
            schema_version: RUNTIME_VERIFICATION_CHALLENGE_SCHEMA_VERSION.to_string(),
            challenge_id: "runtime_challenge_1".to_string(),
            nonce: "burd_runtime_nonce_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            hardware_fingerprint: "a".repeat(64),
            host_os: "linux".to_string(),
            gpu_uuid: "GPU-1111".to_string(),
            runtime_backend: "docker_linux_native".to_string(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            proof_image_ref: format!("ghcr.io/burd/runtime-proof@sha256:{}", "b".repeat(64)),
            proof_policy_version: RUNTIME_PROOF_POLICY_VERSION.to_string(),
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            issued_at: "2026-08-08T00:00:00Z".to_string(),
            expires_at: "2026-08-08T00:10:00Z".to_string(),
            verification_ttl_seconds: 86_400,
        }
    }

    fn evidence(challenge: &RuntimeVerificationChallenge) -> RuntimeVerificationEvidence {
        RuntimeVerificationEvidence {
            host_os: challenge.host_os.clone(),
            runtime_backend: challenge.runtime_backend.clone(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            docker_server_version: "27.1.1".to_string(),
            nvidia_driver_version: "560.35".to_string(),
            nvidia_runtime: "nvidia".to_string(),
            cuda_runtime_version: "12.6".to_string(),
            observed_gpu_uuids: vec![challenge.gpu_uuid.clone()],
            proof_image_digest: challenge.proof_image_ref.clone(),
            proof_nonce: challenge.nonce.clone(),
            network_mode: "none".to_string(),
            run_as_user: "1000:1000".to_string(),
            read_only_rootfs: true,
            no_new_privileges: true,
            cap_drop: vec!["ALL".to_string()],
        }
    }

    #[test]
    fn challenge_requires_digest_pinned_image_and_matching_platform() {
        let mut value = challenge();
        assert!(validate_runtime_verification_challenge(&value).is_ok());
        value.proof_image_ref = "ghcr.io/burd/runtime-proof:latest".to_string();
        assert!(validate_runtime_verification_challenge(&value).is_err());
        value = challenge();
        value.runtime_backend = "docker_wsl2".to_string();
        assert!(validate_runtime_verification_challenge(&value).is_err());
    }

    #[test]
    fn evidence_requires_exact_single_gpu_and_live_nonce() {
        let challenge = challenge();
        let mut value = evidence(&challenge);
        assert!(validate_runtime_verification_evidence(&challenge, &value).is_ok());
        value.observed_gpu_uuids.push("GPU-2222".to_string());
        assert!(validate_runtime_verification_evidence(&challenge, &value).is_err());
        value = evidence(&challenge);
        value.proof_nonce = "old_nonce".to_string();
        assert!(validate_runtime_verification_evidence(&challenge, &value).is_err());
    }

    #[test]
    fn signature_and_fingerprint_bind_runtime_evidence() {
        let challenge = challenge();
        let evidence = evidence(&challenge);
        let fingerprint =
            runtime_verification_fingerprint(&fingerprint_claims(&challenge, &evidence)).unwrap();
        let payload = RuntimeVerificationResponsePayload {
            schema_version: RUNTIME_VERIFICATION_RESPONSE_SCHEMA_VERSION.to_string(),
            challenge_id: challenge.challenge_id.clone(),
            nonce: challenge.nonce.clone(),
            provider_id: challenge.provider_id.clone(),
            device_id: challenge.device_id.clone(),
            session_id: challenge.session_id.clone(),
            hardware_fingerprint: challenge.hardware_fingerprint.clone(),
            gpu_uuid: challenge.gpu_uuid.clone(),
            runtime_backend: challenge.runtime_backend.clone(),
            proof_policy_version: challenge.proof_policy_version.clone(),
            agent_runtime_contract_version: challenge.agent_runtime_contract_version.clone(),
            runtime_verification_fingerprint: fingerprint,
            evidence,
            started_at: "2026-08-08T00:00:01Z".to_string(),
            completed_at: "2026-08-08T00:00:05Z".to_string(),
        };
        let response_hash = runtime_verification_response_hash(&payload).unwrap();
        let key = generate_keypair().unwrap();
        let message =
            runtime_verification_signature_message(&payload, &response_hash, "key_1").unwrap();
        let signature = sign_message(&key.secret_key_base64, message.as_bytes()).unwrap();
        assert!(verify_message(&key.public_key_base64, message.as_bytes(), &signature).unwrap());

        let mut changed = payload.clone();
        changed.evidence.docker_server_version = "28.0.0".to_string();
        let changed_hash = runtime_verification_response_hash(&changed).unwrap();
        let changed_message =
            runtime_verification_signature_message(&changed, &changed_hash, "key_1").unwrap();
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
    fn verified_record_is_rejected_after_ttl() {
        let admission_claims = RuntimeAdmissionFingerprintClaims {
            version: crate::RUNTIME_ADMISSION_FINGERPRINT_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            hardware_fingerprint: "a".repeat(64),
            gpu_uuid: "GPU-1111".to_string(),
            host_os: "linux".to_string(),
            runtime_backend: "docker_linux_native".to_string(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            docker_server_version: "28.3.0".to_string(),
            nvidia_driver_version: "580.1".to_string(),
            nvidia_runtime: "nvidia".to_string(),
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
        };
        let admission_fingerprint = runtime_admission_fingerprint(&admission_claims).unwrap();
        let record = ProviderRuntimeVerificationRecord {
            schema_version: RUNTIME_VERIFICATION_RECORD_SCHEMA_VERSION.to_string(),
            verification_id: "runtime_verification_1".to_string(),
            challenge_id: "runtime_challenge_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            hardware_fingerprint: "a".repeat(64),
            gpu_uuid: "GPU-1111".to_string(),
            host_os: "linux".to_string(),
            runtime_backend: "docker_linux_native".to_string(),
            status: "verified".to_string(),
            gpu_uuid_binding: "verified".to_string(),
            runtime_verification_fingerprint: "b".repeat(64),
            proof_policy_version: RUNTIME_PROOF_POLICY_VERSION.to_string(),
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            proof_image_digest: format!("ghcr.io/burd/runtime-proof@sha256:{}", "c".repeat(64)),
            public_key_id: Some("key_1".to_string()),
            runtime_admission_fingerprint: Some(admission_fingerprint),
            runtime_admission_claims: Some(admission_claims),
            verified_at: "2026-08-08T00:00:00Z".to_string(),
            expires_at: "2026-08-08T01:00:00Z".to_string(),
            reason_codes: Vec::new(),
        };
        assert!(
            validate_provider_runtime_verification_record(
                &record,
                "2026-08-08T00:30:00Z".parse().unwrap()
            )
            .is_ok()
        );
        assert!(
            validate_provider_runtime_verification_record(
                &record,
                "2026-08-08T01:00:00Z".parse().unwrap()
            )
            .is_err()
        );

        let mut legacy = serde_json::to_value(&record).unwrap();
        legacy["schema_version"] = serde_json::json!("burd-runtime-verification-record-v1");
        let legacy = legacy.as_object_mut().unwrap();
        legacy.remove("public_key_id");
        legacy.remove("runtime_admission_fingerprint");
        legacy.remove("runtime_admission_claims");
        let legacy: ProviderRuntimeVerificationRecord =
            serde_json::from_value(serde_json::Value::Object(legacy.clone())).unwrap();
        assert!(
            validate_provider_runtime_verification_record(
                &legacy,
                "2026-08-08T00:30:00Z".parse().unwrap()
            )
            .is_err()
        );
    }
}
