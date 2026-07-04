use crate::challenge::Challenge;
use crate::evidence::EvidenceFreshness;
use crate::identity::AgentIdentityPublic;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSignature {
    pub algorithm: String,
    pub value: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullReport {
    pub identity: Option<AgentIdentityPublic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceFreshness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace_policy: Option<serde_json::Value>,
    pub system: serde_json::Value,
    pub fit: Option<serde_json::Value>,
    pub llm_benchmark: Option<serde_json::Value>,
    pub stability: Option<serde_json::Value>,
    pub network: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_score: Option<serde_json::Value>,
    pub disk: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reliability: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_performance: Option<serde_json::Value>,
    pub score: serde_json::Value,
    pub timestamp: String,
    pub agent_version: String,
    pub benchmark_version: String,
    pub benchmark_profile: String,
    pub challenge: Option<Challenge>,
    pub signature: ReportSignature,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedReport {
    pub provider_id: String,
    pub machine_id: String,
    pub report: FullReport,
    pub report_hash: String,
    pub signature: String,
    pub public_key: String,
    pub key_algorithm: String,
    pub signed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceFreshness>,
    pub signature_valid_locally: bool,
    #[serde(default = "default_canonicalization_version")]
    pub canonicalization_version: String,
}

fn default_canonicalization_version() -> String {
    "burd-json-c14n-v1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReportResult {
    pub report_hash: Option<String>,
    pub signature_valid: bool,
    pub key_algorithm: Option<String>,
    pub provider_id: Option<String>,
    pub machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceFreshness>,
    pub checked_at: String,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_report_serializes() {
        let report = FullReport {
            identity: None,
            evidence: None,
            hardware_fingerprint: None,
            marketplace_policy: None,
            system: serde_json::json!({"os": "linux"}),
            fit: None,
            llm_benchmark: None,
            stability: None,
            network: None,
            network_score: None,
            disk: None,
            reliability: None,
            ai_performance: None,
            score: serde_json::json!({"burd_compute_score": 0}),
            timestamp: "2026-06-08T00:00:00Z".to_string(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: "test".to_string(),
            benchmark_profile: "profile_8gb".to_string(),
            challenge: None,
            signature: ReportSignature {
                algorithm: "placeholder".to_string(),
                value: "placeholder".to_string(),
                status: "mocked".to_string(),
            },
        };
        let signed = SignedReport {
            provider_id: "provider".to_string(),
            machine_id: "machine".to_string(),
            report,
            report_hash: "hash".to_string(),
            signature: "sig".to_string(),
            public_key: "pub".to_string(),
            key_algorithm: "ed25519".to_string(),
            signed_at: "2026-06-08T00:00:00Z".to_string(),
            evidence: None,
            signature_valid_locally: true,
            canonicalization_version: "burd-json-c14n-v1".to_string(),
        };
        let json = serde_json::to_string(&signed).unwrap();
        assert!(json.contains("machine"));
    }
}
