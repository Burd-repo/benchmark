use crate::signature::{canonical_json, verify_message};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub challenge_id: String,
    pub nonce: String,
    pub benchmark_profile: String,
    pub required_tests: Vec<RequiredTest>,
    pub issued_at: String,
    pub expires_at: String,
    pub backend_url: String,
    pub min_agent_version: String,
    pub min_benchmark_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredTest {
    pub name: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResponse {
    pub challenge_id: String,
    pub nonce: String,
    pub provider_id: String,
    pub machine_id: String,
    pub report_hash: String,
    pub signature: String,
    pub public_key: String,
    pub completed_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeVerification {
    pub challenge_id: String,
    pub valid: bool,
    pub signature_valid: bool,
    pub expired: bool,
    pub checked_at: String,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ChallengeResponsePayload<'a> {
    challenge_id: &'a str,
    nonce: &'a str,
    provider_id: &'a str,
    machine_id: &'a str,
    report_hash: &'a str,
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
        expires_at: (issued + Duration::minutes(30)).to_rfc3339(),
        backend_url: "https://api.burd.cloud".to_string(),
        min_agent_version: "0.1.0".to_string(),
        min_benchmark_version: "2026.06-mvp".to_string(),
    }
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

    let expired = challenge_expired(challenge).unwrap_or_else(|error| {
        errors.push(error);
        true
    });
    if expired {
        errors.push("challenge expired".to_string());
    }

    if response.status != "completed" {
        warnings.push(format!("challenge response status is {}", response.status));
    }

    let signature_valid = challenge_response_message(
        &response.challenge_id,
        &response.nonce,
        &response.provider_id,
        &response.machine_id,
        &response.report_hash,
    )
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

    ChallengeVerification {
        challenge_id: challenge.challenge_id.clone(),
        valid: errors.is_empty(),
        signature_valid,
        expired,
        checked_at: Utc::now().to_rfc3339(),
        warnings,
        errors,
    }
}

pub fn challenge_expired(challenge: &Challenge) -> Result<bool, String> {
    let expires = DateTime::parse_from_rfc3339(&challenge.expires_at)
        .map_err(|error| format!("invalid challenge expires_at: {error}"))?
        .with_timezone(&Utc);
    Ok(Utc::now() > expires)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(challenge_expired(&challenge).unwrap());
    }
}
