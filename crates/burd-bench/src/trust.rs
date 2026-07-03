use crate::health::{ReliabilityReport, calculate_reliability, load_reliability_report};
use crate::history::{BenchmarkHistoryList, load_history_list};
use crate::network::{NetworkScoreReport, calculate_network_score, load_network_score_report};
use crate::verification::{ProviderVerification, verify_provider};
use burd_protocol::{ProviderSession, ProviderSessionStatus, load_provider_session};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustScoreReport {
    pub trust_score: f64,
    pub level: String,
    pub status: String,
    pub components: TrustScoreComponents,
    pub verification: TrustVerificationSummary,
    pub history: TrustHistorySummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<TrustSessionSummary>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustScoreComponents {
    pub verification_integrity: f64,
    pub evidence_freshness: f64,
    pub reliability: f64,
    pub network: f64,
    pub history_depth: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustVerificationSummary {
    pub signed_report_current: bool,
    pub challenge_verified: bool,
    pub fingerprint_matches: bool,
    pub audit_status: String,
    pub fraud_risk_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustHistorySummary {
    pub entries_total: usize,
    pub latest_signed: bool,
    pub latest_challenge: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustSessionSummary {
    pub status: String,
    pub online_locally: bool,
    pub heartbeat_count: u64,
}

pub fn build_trust_score(agent_version: &str) -> TrustScoreReport {
    let verification = verify_provider(agent_version);
    let reliability = load_reliability_report().unwrap_or_else(|_| calculate_reliability(&[]));
    let network = load_network_score_report().unwrap_or_else(|_| calculate_network_score(None));
    let history = load_history_list().ok();
    let session = load_provider_session().ok().flatten();
    calculate_trust_score(
        &verification,
        &reliability,
        &network,
        history.as_ref(),
        session.as_ref(),
    )
}

pub fn calculate_trust_score(
    verification: &ProviderVerification,
    reliability: &ReliabilityReport,
    network: &NetworkScoreReport,
    history: Option<&BenchmarkHistoryList>,
    session: Option<&ProviderSession>,
) -> TrustScoreReport {
    let verification_integrity = verification_integrity_score(verification);
    let evidence_freshness = evidence_freshness_score(verification);
    let reliability_component = reliability.reliability_score.clamp(0.0, 100.0);
    let network_component = network.network_score.clamp(0.0, 100.0);
    let history_component = history_depth_score(history);

    let trust_score = round1(
        (verification_integrity * 0.40
            + evidence_freshness * 0.20
            + reliability_component * 0.20
            + network_component * 0.10
            + history_component * 0.10)
            .clamp(0.0, 100.0),
    );

    let mut warnings = Vec::new();
    if verification.fraud_risk_level == "high" {
        warnings.push("provider verification reports high fraud risk".to_string());
    }
    if !verification.fingerprint_matches {
        warnings.push(
            "current hardware fingerprint does not match the latest signed evidence".to_string(),
        );
    }
    if !verification.signed_report_current {
        warnings.push("latest signed report is missing or expired".to_string());
    }
    if !verification.challenge_verified {
        warnings.push("no current locally verified challenge evidence is available".to_string());
    }
    if reliability.checks_total == 0 {
        warnings.push("no heartbeat history is available yet".to_string());
    } else if reliability.status != "reliable" {
        warnings.push(format!(
            "local reliability status is {}",
            reliability.status
        ));
    }
    if network.status == "no_benchmark" {
        warnings.push("no finite local network benchmark is available".to_string());
    } else if network.status != "ready" {
        warnings.push(format!("network score status is {}", network.status));
    }
    if history.is_none_or(|value| value.entries_total == 0) {
        warnings.push("benchmark history is empty".to_string());
    }
    if let Some(session) = session
        && session.status != ProviderSessionStatus::Active
    {
        warnings.push(format!(
            "local provider session is {}",
            session.status.as_str()
        ));
    }
    deduplicate(&mut warnings);

    TrustScoreReport {
        trust_score,
        level: trust_level(trust_score).to_string(),
        status: trust_status(trust_score, verification, reliability, history, session).to_string(),
        components: TrustScoreComponents {
            verification_integrity: round1(verification_integrity),
            evidence_freshness: round1(evidence_freshness),
            reliability: round1(reliability_component),
            network: round1(network_component),
            history_depth: round1(history_component),
        },
        verification: TrustVerificationSummary {
            signed_report_current: verification.signed_report_current,
            challenge_verified: verification.challenge_verified,
            fingerprint_matches: verification.fingerprint_matches,
            audit_status: verification.audit_status.clone(),
            fraud_risk_level: verification.fraud_risk_level.clone(),
        },
        history: TrustHistorySummary {
            entries_total: history.map(|value| value.entries_total).unwrap_or(0),
            latest_signed: history
                .and_then(|value| value.entries.last())
                .map(|entry| entry.signed)
                .unwrap_or(false),
            latest_challenge: history
                .and_then(|value| value.entries.last())
                .and_then(|entry| entry.challenge_id.as_ref())
                .is_some(),
        },
        session: session.map(|value| TrustSessionSummary {
            status: value.status.as_str().to_string(),
            online_locally: value.online_locally,
            heartbeat_count: value.heartbeat_count,
        }),
        warnings,
        notes: vec![
            "Trust score is a local heuristic only.".to_string(),
            "It summarizes local verification integrity, evidence freshness, reliability, network quality, and benchmark history depth.".to_string(),
            "Trust score is not backend approval, marketplace admission, public SLA, or payout eligibility.".to_string(),
        ],
    }
}

fn verification_integrity_score(verification: &ProviderVerification) -> f64 {
    let mut score: f64 = 0.0;
    if verification.hardware_verified {
        score += 20.0;
    }
    if verification.signature_verified {
        score += 20.0;
    }
    if verification.fingerprint_matches {
        score += 20.0;
    }
    if verification.benchmark_verified {
        score += 15.0;
    }
    if verification.challenge_verified {
        score += 15.0;
    }
    if verification.audit_status == "self_verified" {
        score += 10.0;
    }

    match verification.fraud_risk_level.as_str() {
        "high" => 0.0,
        "medium" => (score - 15.0).max(0.0),
        _ => score.clamp(0.0, 100.0),
    }
}

fn evidence_freshness_score(verification: &ProviderVerification) -> f64 {
    let mut score: f64 = 0.0;
    if verification.signed_report_current {
        score += 60.0;
    } else if verification.signed_report_evidence.is_some() {
        score += 20.0;
    }
    if verification.challenge_verified {
        score += 40.0;
    } else if verification.challenge_evidence.is_some() {
        score += 10.0;
    }
    score.clamp(0.0, 100.0)
}

fn history_depth_score(history: Option<&BenchmarkHistoryList>) -> f64 {
    let Some(history) = history else {
        return 0.0;
    };
    match history.entries_total {
        0 => 0.0,
        1 => 35.0,
        2..=4 => 65.0,
        5..=9 => 85.0,
        _ => 100.0,
    }
}

fn trust_level(score: f64) -> &'static str {
    match score {
        value if value >= 90.0 => "High",
        value if value >= 70.0 => "Established",
        value if value >= 45.0 => "Developing",
        _ => "Low",
    }
}

fn trust_status(
    score: f64,
    verification: &ProviderVerification,
    reliability: &ReliabilityReport,
    history: Option<&BenchmarkHistoryList>,
    session: Option<&ProviderSession>,
) -> &'static str {
    if verification.fraud_risk_level == "high" || !verification.fingerprint_matches {
        return "at_risk";
    }
    if !verification.signed_report_current {
        return "stale_evidence";
    }
    if history.is_none_or(|value| value.entries_total == 0) || reliability.checks_total == 0 {
        return "warming_up";
    }
    if session.is_some_and(|value| value.status == ProviderSessionStatus::Invalidated) {
        return "session_invalid";
    }
    if score >= 70.0 {
        "trusted_locally"
    } else {
        "limited"
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn deduplicate(items: &mut Vec<String>) {
    items.sort();
    items.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_score_rewards_verified_fresh_history() {
        let verification = crate::test_fixtures::provider_verification();
        let reliability = crate::test_fixtures::reliability_report();
        let network = crate::test_fixtures::network_score_report();
        let history = crate::test_fixtures::history_list();

        let report = calculate_trust_score(
            &verification,
            &reliability,
            &network,
            Some(&history),
            None,
        );

        assert!(report.trust_score >= 70.0);
        assert_eq!(report.status, "trusted_locally");
        assert_eq!(report.history.entries_total, history.entries_total);
    }

    #[test]
    fn trust_score_flags_high_risk_and_mismatch() {
        let mut verification = crate::test_fixtures::provider_verification();
        verification.fraud_risk_level = "high".to_string();
        verification.fingerprint_matches = false;

        let report = calculate_trust_score(
            &verification,
            &crate::test_fixtures::reliability_report(),
            &crate::test_fixtures::network_score_report(),
            None,
            None,
        );

        assert_eq!(report.status, "at_risk");
        assert!(report.warnings.iter().any(|item| item.contains("fraud risk")));
        assert!(report.warnings.iter().any(|item| item.contains("fingerprint")));
    }
}
