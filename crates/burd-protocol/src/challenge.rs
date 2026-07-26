use crate::identity::default_state_dir;
use crate::local_state::write_json_atomic;
use crate::report::SignedReport;
use crate::signature::{KEY_ALGORITHM, canonical_json, hash_canonical, verify_message};
use crate::{CHALLENGE_TTL_SECONDS, EvidenceFreshness, evidence_freshness_from_window};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use uuid::Uuid;

pub const PROOF_CHALLENGE_SCHEMA_VERSION: &str = "burd-proof-capability-challenge-v1";
pub const PROOF_CHALLENGE_RESPONSE_SCHEMA_VERSION: &str = "burd-proof-capability-response-v1";
pub const PROOF_CHALLENGE_CANONICALIZATION_VERSION: &str = "burd-json-c14n-v1";
pub const PROOF_CHALLENGE_SIGNATURE_DOMAIN: &str = "burd.proof-capability-response.v1";
pub const VERIFICATION_POLICY_VERSION: &str = "burd-verification-policy-v1";
pub const PROOF_CAPABILITY_REQUIRED_PROOFS: &[&str] = &[
    "cuda_runtime",
    "vram_allocation_residency",
    "tensor_gemm_microbenchmark",
    "llm_short_inference",
    "performance_consistency",
    "contention_detection",
    "telemetry_window",
];
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub challenge_id: String,
    pub nonce: String,
    pub benchmark_profile: String,
    pub required_tests: Vec<RequiredTest>,
    pub issued_at: String,
    pub expires_at: String,
    #[serde(default)]
    pub is_expired: bool,
    #[serde(default)]
    pub age_seconds: u64,
    #[serde(default)]
    pub ttl_seconds: u64,
    #[serde(default)]
    pub backend_url: Option<String>,
    pub min_agent_version: String,
    pub min_benchmark_version: String,
    #[serde(default)]
    pub policy: ChallengePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredTest {
    pub name: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengePolicy {
    pub require_signed_report: bool,
    pub require_llm_benchmark: bool,
    pub require_stability: bool,
    pub require_network: bool,
    pub require_disk: bool,
}

impl Default for ChallengePolicy {
    fn default() -> Self {
        Self {
            require_signed_report: true,
            require_llm_benchmark: true,
            require_stability: true,
            require_network: true,
            require_disk: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResponse {
    pub challenge_id: String,
    pub nonce: String,
    pub provider_id: String,
    pub machine_id: String,
    pub report_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_fingerprint: Option<String>,
    #[serde(default)]
    pub signed_report: Option<SignedReport>,
    pub signature: String,
    pub public_key: String,
    pub completed_at: String,
    pub issued_at: String,
    pub expires_at: String,
    pub is_expired: bool,
    pub age_seconds: u64,
    pub ttl_seconds: u64,
    pub status: String,
    #[serde(default)]
    pub failed_requirements: Vec<String>,
    #[serde(default)]
    pub verification_result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeVerification {
    pub challenge_id: String,
    pub valid: bool,
    pub signature_valid: bool,
    pub expired: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceFreshness>,
    pub checked_at: String,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeRunOutput {
    pub challenge: Challenge,
    pub signed_report: SignedReport,
    pub response: ChallengeResponse,
    pub verification: ChallengeVerification,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueProofChallengeRequest {
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub profile_version: String,
    pub required_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_gpu_uuid: Option<String>,
    pub required_backend: String,
    pub model_artifact_hash: String,
    pub prompt_seed: String,
    #[serde(default)]
    pub required_proofs: Vec<String>,
    pub min_tokens_per_second: f64,
    pub max_ttft_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofCapabilityChallenge {
    pub schema_version: String,
    pub challenge_id: String,
    pub nonce: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub profile_version: String,
    pub required_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_gpu_uuid: Option<String>,
    pub required_backend: String,
    pub model_artifact_hash: String,
    pub prompt_seed: String,
    #[serde(default)]
    pub required_proofs: Vec<String>,
    pub min_tokens_per_second: f64,
    pub max_ttft_ms: u64,
    pub issued_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofCapabilityMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_allocated_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_resident_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemm_gflops: Option<f64>,
    #[serde(default)]
    pub cuda_runtime_detected: bool,
    pub backend_proof: String,
    #[serde(default)]
    pub contention_detected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofCapabilityResponsePayload {
    pub schema_version: String,
    pub challenge_id: String,
    pub nonce: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub profile_version: String,
    pub hardware_fingerprint: String,
    pub gpu_uuid: String,
    pub backend: String,
    pub model_artifact_hash: String,
    pub prompt_seed: String,
    pub driver_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_driver_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_runtime_version: Option<String>,
    pub metrics: ProofCapabilityMetrics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_window_hash: Option<String>,
    pub started_at: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedProofCapabilityResponse {
    pub payload: ProofCapabilityResponsePayload,
    pub response_hash: String,
    pub public_key_id: String,
    pub signature: String,
    pub canonicalization_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofChallengeVerification {
    pub schema_version: String,
    pub challenge_id: String,
    pub checked_at: String,
    pub response_hash_valid: bool,
    pub signature_valid: bool,
    pub provider_bound: bool,
    pub device_bound: bool,
    pub session_bound: bool,
    pub fingerprint_bound: bool,
    pub gpu_bound: bool,
    pub backend_bound: bool,
    pub artifact_bound: bool,
    pub prompt_bound: bool,
    pub metrics_satisfied: bool,
    pub expired_by_server: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofChallengeRecord {
    pub challenge: ProofCapabilityChallenge,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_object_key: Option<String>,
    pub issued_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expired_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<ProofChallengeVerification>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueProofChallengeResponse {
    pub request_id: String,
    pub challenge: ProofCapabilityChallenge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextProofChallengeResponse {
    pub request_id: String,
    pub challenge: ProofCapabilityChallenge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitProofChallengeResponse {
    pub request_id: String,
    pub challenge_id: String,
    pub status: String,
    pub response_hash: String,
    pub server_received_at: String,
    pub verification: ProofChallengeVerification,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunVerificationSweepRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default)]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationSweepIssuedChallenge {
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub challenge_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunVerificationSweepResponse {
    pub request_id: String,
    pub evaluated: u32,
    pub issued: Vec<VerificationSweepIssuedChallenge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationStateRecord {
    pub provider_id: String,
    pub device_id: String,
    pub status: String,
    pub policy_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub risk_score: f64,
    pub success_count: i32,
    pub failure_count: i32,
    pub retry_budget_remaining: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_challenge_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified_challenge_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_due_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quarantined_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListVerificationStatesResponse {
    pub request_id: String,
    pub states: Vec<VerificationStateRecord>,
}

#[derive(Debug, Clone, Serialize)]
struct ChallengeResponsePayload<'a> {
    challenge_id: &'a str,
    nonce: &'a str,
    provider_id: &'a str,
    machine_id: &'a str,
    report_hash: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct ChallengeResponsePayloadWithFingerprint<'a> {
    challenge_id: &'a str,
    nonce: &'a str,
    provider_id: &'a str,
    machine_id: &'a str,
    report_hash: &'a str,
    hardware_fingerprint: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct ProofChallengeResponseSignatureClaims<'a> {
    domain: &'static str,
    response_hash: &'a str,
    challenge_id: &'a str,
    nonce: &'a str,
    provider_id: &'a str,
    device_id: &'a str,
    session_id: &'a str,
    profile_version: &'a str,
    hardware_fingerprint: &'a str,
    gpu_uuid: &'a str,
    backend: &'a str,
    model_artifact_hash: &'a str,
    prompt_seed: &'a str,
    public_key_id: &'a str,
}
pub fn mock_challenge(profile: &str) -> Challenge {
    let issued = Utc::now();
    Challenge {
        challenge_id: format!("challenge-{}", Uuid::new_v4()),
        nonce: Uuid::new_v4().to_string(),
        benchmark_profile: profile.to_string(),
        required_tests: vec![
            RequiredTest {
                name: "system".to_string(),
                required: true,
            },
            RequiredTest {
                name: "fit".to_string(),
                required: true,
            },
            RequiredTest {
                name: "llm_benchmark".to_string(),
                required: true,
            },
            RequiredTest {
                name: "stability".to_string(),
                required: true,
            },
            RequiredTest {
                name: "network".to_string(),
                required: false,
            },
            RequiredTest {
                name: "disk".to_string(),
                required: false,
            },
        ],
        issued_at: issued.to_rfc3339(),
        expires_at: (issued + Duration::seconds(CHALLENGE_TTL_SECONDS as i64)).to_rfc3339(),
        is_expired: false,
        age_seconds: 0,
        ttl_seconds: CHALLENGE_TTL_SECONDS,
        backend_url: Some("https://api.burd.cloud".to_string()),
        min_agent_version: "0.1.0".to_string(),
        min_benchmark_version: "2026.06-mvp".to_string(),
        policy: ChallengePolicy::default(),
    }
}

pub fn proof_capability_response_hash(
    payload: &ProofCapabilityResponsePayload,
) -> Result<String, String> {
    hash_canonical(payload)
}

pub fn proof_capability_response_signature_message(
    payload: &ProofCapabilityResponsePayload,
    response_hash: &str,
    public_key_id: &str,
) -> Result<String, String> {
    canonical_json(&ProofChallengeResponseSignatureClaims {
        domain: PROOF_CHALLENGE_SIGNATURE_DOMAIN,
        response_hash,
        challenge_id: &payload.challenge_id,
        nonce: &payload.nonce,
        provider_id: &payload.provider_id,
        device_id: &payload.device_id,
        session_id: &payload.session_id,
        profile_version: &payload.profile_version,
        hardware_fingerprint: &payload.hardware_fingerprint,
        gpu_uuid: &payload.gpu_uuid,
        backend: &payload.backend,
        model_artifact_hash: &payload.model_artifact_hash,
        prompt_seed: &payload.prompt_seed,
        public_key_id,
    })
}

pub fn challenge_response_message(
    challenge_id: &str,
    nonce: &str,
    provider_id: &str,
    machine_id: &str,
    report_hash: &str,
) -> Result<String, String> {
    canonical_json(&ChallengeResponsePayload {
        challenge_id,
        nonce,
        provider_id,
        machine_id,
        report_hash,
    })
}

pub fn challenge_response_message_with_fingerprint(
    challenge_id: &str,
    nonce: &str,
    provider_id: &str,
    machine_id: &str,
    report_hash: &str,
    hardware_fingerprint: &str,
) -> Result<String, String> {
    canonical_json(&ChallengeResponsePayloadWithFingerprint {
        challenge_id,
        nonce,
        provider_id,
        machine_id,
        report_hash,
        hardware_fingerprint,
    })
}

pub fn verify_challenge_response(
    challenge: &Challenge,
    response: &ChallengeResponse,
) -> ChallengeVerification {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    if challenge.challenge_id != response.challenge_id {
        errors.push("challenge_id mismatch".to_string());
    }
    if challenge.nonce != response.nonce {
        errors.push("nonce mismatch".to_string());
    }

    let evidence = evidence_freshness_from_window(&challenge.issued_at, &challenge.expires_at)
        .map_err(|error| {
            errors.push(error);
        })
        .ok();
    let expired = evidence.as_ref().is_none_or(|evidence| evidence.is_expired);
    if expired {
        errors.push("challenge expired".to_string());
    }

    if !matches!(
        response.status.as_str(),
        "completed" | "passed" | "failed" | "expired" | "partial"
    ) {
        warnings.push(format!("challenge response status is {}", response.status));
    }

    let signature_message = match response.hardware_fingerprint.as_deref() {
        Some(fingerprint) => challenge_response_message_with_fingerprint(
            &response.challenge_id,
            &response.nonce,
            &response.provider_id,
            &response.machine_id,
            &response.report_hash,
            fingerprint,
        ),
        None => challenge_response_message(
            &response.challenge_id,
            &response.nonce,
            &response.provider_id,
            &response.machine_id,
            &response.report_hash,
        ),
    };
    let signature_valid = signature_message
        .and_then(|message| {
            verify_message(
                &response.public_key,
                message.as_bytes(),
                &response.signature,
            )
        })
        .unwrap_or_else(|error| {
            errors.push(error);
            false
        });
    if !signature_valid {
        errors.push("challenge response signature invalid".to_string());
    }

    let mut failed_requirements = validate_required_tests(challenge, response, &mut warnings);
    errors.append(&mut failed_requirements);

    ChallengeVerification {
        challenge_id: challenge.challenge_id.clone(),
        valid: errors.is_empty(),
        signature_valid,
        expired,
        evidence,
        checked_at: Utc::now().to_rfc3339(),
        warnings,
        errors,
    }
}

pub fn save_latest_challenge_output(output: &ChallengeRunOutput) -> Result<(), String> {
    let path = default_state_dir().join("latest-challenge-response.json");
    write_json_atomic(&path, output)
        .map_err(|error| format!("failed to persist latest challenge output: {error}"))
}

pub fn load_latest_challenge_output() -> Result<ChallengeRunOutput, String> {
    let path = default_state_dir().join("latest-challenge-response.json");
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&raw).map_err(|error| format!("invalid challenge output JSON: {error}"))
}

fn validate_required_tests(
    challenge: &Challenge,
    response: &ChallengeResponse,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(signed_report) = response.signed_report.as_ref() else {
        if challenge.policy.require_signed_report {
            errors.push("signed report required by challenge policy".to_string());
        }
        return errors;
    };

    if signed_report.report_hash != response.report_hash {
        errors.push("response report_hash does not match signed_report report_hash".to_string());
    }
    if signed_report.provider_id != response.provider_id {
        errors.push("response provider_id does not match signed report".to_string());
    }
    if signed_report.machine_id != response.machine_id {
        errors.push("response machine_id does not match signed report".to_string());
    }
    match (
        response.hardware_fingerprint.as_deref(),
        signed_report.report.hardware_fingerprint.as_deref(),
    ) {
        (Some(response_fingerprint), Some(report_fingerprint))
            if response_fingerprint != report_fingerprint =>
        {
            errors.push("response hardware_fingerprint does not match signed report".to_string());
        }
        (None, Some(_)) => {
            errors.push("response hardware_fingerprint missing".to_string());
        }
        (Some(_), None) => {
            errors.push("signed report hardware_fingerprint missing".to_string());
        }
        (None, None) => warnings.push(
            "challenge response and signed report do not include hardware fingerprint".to_string(),
        ),
        _ => {}
    }
    if signed_report.key_algorithm != KEY_ALGORITHM {
        errors.push(format!(
            "unsupported signed report key algorithm '{}'",
            signed_report.key_algorithm
        ));
    }

    match hash_canonical(&signed_report.report) {
        Ok(hash) if hash == signed_report.report_hash => {}
        Ok(_) => errors.push("signed report hash does not match canonical report".to_string()),
        Err(error) => errors.push(error),
    }
    match verify_message(
        &signed_report.public_key,
        signed_report.report_hash.as_bytes(),
        &signed_report.signature,
    ) {
        Ok(true) => {}
        Ok(false) => errors.push("signed report signature invalid".to_string()),
        Err(error) => errors.push(error),
    }

    if signed_report.report.agent_version.as_str() < challenge.min_agent_version.as_str() {
        errors.push(format!(
            "agent version {} is below challenge minimum {}",
            signed_report.report.agent_version, challenge.min_agent_version
        ));
    }
    if signed_report.report.benchmark_version.as_str() < challenge.min_benchmark_version.as_str() {
        errors.push(format!(
            "benchmark version {} is below challenge minimum {}",
            signed_report.report.benchmark_version, challenge.min_benchmark_version
        ));
    }

    let required_names: Vec<&str> = challenge
        .required_tests
        .iter()
        .filter(|test| test.required)
        .map(|test| test.name.as_str())
        .collect();
    for name in required_names {
        if !report_has_test(signed_report, name) {
            errors.push(format!("required test missing from signed report: {name}"));
        }
    }

    if challenge.policy.require_llm_benchmark && !report_has_test(signed_report, "llm_benchmark") {
        errors.push("policy requires llm_benchmark".to_string());
    }
    if challenge.policy.require_stability && !report_has_test(signed_report, "stability") {
        errors.push("policy requires stability".to_string());
    }
    if challenge.policy.require_network && !report_has_test(signed_report, "network") {
        errors.push("policy requires network".to_string());
    }
    if challenge.policy.require_disk && !report_has_test(signed_report, "disk") {
        errors.push("policy requires disk".to_string());
    }

    for item in &response.failed_requirements {
        warnings.push(format!("response declared failed requirement: {item}"));
    }

    errors
}

fn report_has_test(signed_report: &SignedReport, name: &str) -> bool {
    match name {
        "system" => !signed_report.report.system.is_null(),
        "fit" => signed_report.report.fit.as_ref().is_some_and(not_skipped),
        "llm_benchmark" => signed_report
            .report
            .llm_benchmark
            .as_ref()
            .is_some_and(not_skipped),
        "stability" => signed_report
            .report
            .stability
            .as_ref()
            .is_some_and(not_skipped),
        "network" => signed_report
            .report
            .network
            .as_ref()
            .is_some_and(not_skipped),
        "disk" => signed_report.report.disk.as_ref().is_some_and(not_skipped),
        other => signed_report
            .report
            .score
            .get(other)
            .is_some_and(|value| !value.is_null()),
    }
}

fn not_skipped(value: &serde_json::Value) -> bool {
    value
        .get("status")
        .and_then(|status| status.as_str())
        .map(|status| status != "skipped")
        .unwrap_or(true)
}

pub fn challenge_expired(challenge: &Challenge) -> Result<bool, String> {
    Ok(evidence_freshness_from_window(&challenge.issued_at, &challenge.expires_at)?.is_expired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{FullReport, ReportSignature};
    use crate::signature::{generate_keypair, hash_canonical, sign_message};

    #[test]
    fn challenge_serializes() {
        let challenge = mock_challenge("profile_24gb");
        let json = serde_json::to_string(&challenge).unwrap();
        assert!(json.contains("challenge_id"));
        assert!(json.contains("profile_24gb"));
        assert!(json.contains("min_agent_version"));
    }

    #[test]
    fn expired_challenge_is_detected() {
        let mut challenge = mock_challenge("profile_8gb");
        challenge.expires_at = (Utc::now() - Duration::seconds(1)).to_rfc3339();
        challenge.issued_at = (Utc::now() - Duration::hours(25)).to_rfc3339();
        assert!(challenge_expired(&challenge).unwrap());
    }

    #[test]
    fn required_tests_missing_from_signed_report_fail_validation() {
        let keys = generate_keypair().unwrap();
        let challenge = Challenge {
            required_tests: vec![RequiredTest {
                name: "llm_benchmark".to_string(),
                required: true,
            }],
            policy: ChallengePolicy {
                require_llm_benchmark: false,
                require_stability: false,
                require_network: false,
                require_disk: false,
                ..ChallengePolicy::default()
            },
            ..mock_challenge("profile_8gb")
        };
        let mut report = FullReport {
            identity: None,
            evidence: None,
            hardware_fingerprint: None,
            marketplace_policy: None,
            system: serde_json::json!({"os": "linux"}),
            fit: Some(serde_json::json!({"status": "ok"})),
            llm_benchmark: Some(serde_json::json!({"status": "skipped"})),
            stability: None,
            network: None,
            network_score: None,
            disk: None,
            reliability: None,
            ai_performance: None,
            score: serde_json::json!({"burd_compute_score": 0, "tier": "Not Eligible"}),
            timestamp: Utc::now().to_rfc3339(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: "2026.06-mvp".to_string(),
            benchmark_profile: "profile_8gb".to_string(),
            challenge: Some(challenge.clone()),
            signature: ReportSignature {
                algorithm: KEY_ALGORITHM.to_string(),
                value: "signature-in-envelope".to_string(),
                status: "signed".to_string(),
            },
        };
        let report_hash = hash_canonical(&report).unwrap();
        report.signature.value = "signature-in-envelope".to_string();
        let signature = sign_message(&keys.secret_key_base64, report_hash.as_bytes()).unwrap();
        let signed_report = SignedReport {
            provider_id: "provider".to_string(),
            machine_id: "machine".to_string(),
            report,
            report_hash: report_hash.clone(),
            signature,
            public_key: keys.public_key_base64.clone(),
            key_algorithm: KEY_ALGORITHM.to_string(),
            signed_at: Utc::now().to_rfc3339(),
            evidence: None,
            signature_valid_locally: true,
            canonicalization_version: "burd-json-c14n-v1".to_string(),
        };
        let message = challenge_response_message(
            &challenge.challenge_id,
            &challenge.nonce,
            "provider",
            "machine",
            &report_hash,
        )
        .unwrap();
        let response = ChallengeResponse {
            challenge_id: challenge.challenge_id.clone(),
            nonce: challenge.nonce.clone(),
            provider_id: "provider".to_string(),
            machine_id: "machine".to_string(),
            report_hash,
            hardware_fingerprint: None,
            signed_report: Some(signed_report),
            signature: sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap(),
            public_key: keys.public_key_base64,
            completed_at: Utc::now().to_rfc3339(),
            issued_at: Utc::now().to_rfc3339(),
            expires_at: challenge.expires_at.clone(),
            is_expired: false,
            age_seconds: 0,
            ttl_seconds: challenge.ttl_seconds,
            status: "partial".to_string(),
            failed_requirements: Vec::new(),
            verification_result: None,
        };

        let result = verify_challenge_response(&challenge, &response);
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("required test missing"))
        );
    }

    fn proof_payload() -> ProofCapabilityResponsePayload {
        ProofCapabilityResponsePayload {
            schema_version: PROOF_CHALLENGE_RESPONSE_SCHEMA_VERSION.to_string(),
            challenge_id: "challenge_1".to_string(),
            nonce: "nonce_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            profile_version: "poc-cuda-llm-v1".to_string(),
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            backend: "cuda".to_string(),
            model_artifact_hash: "sha256:model".to_string(),
            prompt_seed: "seed_1".to_string(),
            driver_version: "576.80".to_string(),
            cuda_driver_version: Some("12.9".to_string()),
            cuda_runtime_version: Some("12.8".to_string()),
            metrics: ProofCapabilityMetrics {
                tokens_per_second: Some(42.0),
                ttft_ms: Some(120),
                vram_allocated_mib: Some(4096),
                vram_resident_mib: Some(4096),
                gemm_gflops: Some(9500.0),
                cuda_runtime_detected: true,
                backend_proof: "cuda-device-query".to_string(),
                contention_detected: false,
            },
            telemetry_window_hash: Some("sha256:telemetry-window".to_string()),
            started_at: "2026-07-09T00:00:00Z".to_string(),
            completed_at: "2026-07-09T00:00:05Z".to_string(),
        }
    }

    #[test]
    fn proof_capability_hash_changes_with_payload() {
        let payload = proof_payload();
        let hash = proof_capability_response_hash(&payload).unwrap();
        let mut changed = payload.clone();
        changed.gpu_uuid = "GPU-other".to_string();
        let changed_hash = proof_capability_response_hash(&changed).unwrap();

        assert_ne!(hash, changed_hash);
    }

    #[test]
    fn proof_capability_signature_binds_payload() {
        let keys = generate_keypair().unwrap();
        let payload = proof_payload();
        let hash = proof_capability_response_hash(&payload).unwrap();
        let message =
            proof_capability_response_signature_message(&payload, &hash, "key_1").unwrap();
        let signature = sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap();

        assert!(
            crate::signature::verify_message(
                &keys.public_key_base64,
                message.as_bytes(),
                &signature
            )
            .unwrap()
        );

        let mut changed = payload.clone();
        changed.hardware_fingerprint = "sha256:other".to_string();
        let changed_message =
            proof_capability_response_signature_message(&changed, &hash, "key_1").unwrap();
        assert!(
            !crate::signature::verify_message(
                &keys.public_key_base64,
                changed_message.as_bytes(),
                &signature,
            )
            .unwrap()
        );
    }
}
