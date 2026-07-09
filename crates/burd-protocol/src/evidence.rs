use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::report::SignedReport;

pub const FULL_REPORT_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const SIGNED_REPORT_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const CHALLENGE_TTL_SECONDS: u64 = 24 * 60 * 60;
pub const EVIDENCE_REGISTRY_SCHEMA_VERSION: &str = "burd.evidence-registry.v1";
pub const EVIDENCE_CANONICALIZATION_VERSION: &str = "burd-json-c14n-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceFreshness {
    pub issued_at: String,
    pub expires_at: String,
    pub is_expired: bool,
    pub age_seconds: u64,
    pub ttl_seconds: u64,
}

pub fn evidence_freshness(issued_at: &str, ttl_seconds: u64) -> Result<EvidenceFreshness, String> {
    evidence_freshness_at(issued_at, ttl_seconds, Utc::now())
}

pub fn evidence_freshness_at(
    issued_at: &str,
    ttl_seconds: u64,
    now: DateTime<Utc>,
) -> Result<EvidenceFreshness, String> {
    let issued = parse_rfc3339(issued_at, "issued_at")?;
    let expires = issued + Duration::seconds(ttl_seconds as i64);
    Ok(build_freshness(issued, expires, now))
}

pub fn evidence_freshness_from_window(
    issued_at: &str,
    expires_at: &str,
) -> Result<EvidenceFreshness, String> {
    evidence_freshness_from_window_at(issued_at, expires_at, Utc::now())
}

pub fn evidence_freshness_from_window_at(
    issued_at: &str,
    expires_at: &str,
    now: DateTime<Utc>,
) -> Result<EvidenceFreshness, String> {
    let issued = parse_rfc3339(issued_at, "issued_at")?;
    let expires = parse_rfc3339(expires_at, "expires_at")?;
    if expires < issued {
        return Err("expires_at is before issued_at".to_string());
    }
    Ok(build_freshness(issued, expires, now))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitEvidenceRequest {
    #[serde(default = "default_evidence_type")]
    pub evidence_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub signed_report: SignedReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceVerification {
    pub schema_version: String,
    pub checked_at: String,
    pub report_hash_valid: bool,
    pub evidence_hash_valid: bool,
    pub signature_valid: bool,
    pub active_key_bound: bool,
    pub provider_bound: bool,
    pub device_bound: bool,
    pub fingerprint_bound: bool,
    pub expired_by_server: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_freshness: Option<EvidenceFreshness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_envelope_claimed_expired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_report_claimed_expired: Option<bool>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub evidence_id: String,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub evidence_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    pub canonicalization_version: String,
    pub evidence_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_key: Option<String>,
    pub status: String,
    pub server_received_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_reason: Option<String>,
    pub verification: EvidenceVerification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitEvidenceResponse {
    pub request_id: String,
    pub duplicate: bool,
    pub evidence: EvidenceRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEvidenceResponse {
    pub request_id: String,
    pub provider_id: String,
    pub records: Vec<EvidenceRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokeEvidenceRequest {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokeEvidenceResponse {
    pub request_id: String,
    pub evidence_id: String,
    pub status: String,
    pub revoked_at: String,
    pub reason: String,
}

fn parse_rfc3339(value: &str, field: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| format!("invalid evidence {field}: {error}"))
}

fn build_freshness(
    issued: DateTime<Utc>,
    expires: DateTime<Utc>,
    now: DateTime<Utc>,
) -> EvidenceFreshness {
    EvidenceFreshness {
        issued_at: issued.to_rfc3339(),
        expires_at: expires.to_rfc3339(),
        is_expired: now > expires,
        age_seconds: now.signed_duration_since(issued).num_seconds().max(0) as u64,
        ttl_seconds: expires.signed_duration_since(issued).num_seconds().max(0) as u64,
    }
}

fn default_evidence_type() -> String {
    "signed_report".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_marks_valid_and_expired_windows() {
        let issued = DateTime::parse_from_rfc3339("2026-06-08T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let valid =
            evidence_freshness_at("2026-06-08T00:00:00Z", 86_400, issued + Duration::hours(12))
                .unwrap();
        assert!(!valid.is_expired);
        assert_eq!(valid.age_seconds, 43_200);
        assert_eq!(valid.ttl_seconds, 86_400);

        let expired =
            evidence_freshness_at("2026-06-08T00:00:00Z", 86_400, issued + Duration::hours(25))
                .unwrap();
        assert!(expired.is_expired);
    }
}
