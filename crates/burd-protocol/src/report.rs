use crate::challenge::Challenge;
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
    pub system: serde_json::Value,
    pub fit: Option<serde_json::Value>,
    pub llm_benchmark: Option<serde_json::Value>,
    pub stability: Option<serde_json::Value>,
    pub network: Option<serde_json::Value>,
    pub disk: Option<serde_json::Value>,
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
    pub provider_id: Option<String>,
    pub machine_id: String,
    pub challenge_id: String,
    pub nonce: String,
    pub report: FullReport,
    pub signature: String,
    pub public_key: String,
    pub generated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_report_serializes() {
        let report = FullReport {
            identity: None,
            system: serde_json::json!({"os": "linux"}),
            fit: None,
            llm_benchmark: None,
            stability: None,
            network: None,
            disk: None,
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
            provider_id: None,
            machine_id: "machine".to_string(),
            challenge_id: "challenge".to_string(),
            nonce: "nonce".to_string(),
            report,
            signature: "sig".to_string(),
            public_key: "pub".to_string(),
            generated_at: "2026-06-08T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&signed).unwrap();
        assert!(json.contains("machine"));
    }
}
