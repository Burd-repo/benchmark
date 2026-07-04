use crate::actions::{load_actions, load_logs, logs_summary};
use crate::health::load_uptime_summary;
use crate::history::history_summary;
use crate::provider::build_provider_details;
use crate::report::{load_latest_signed_report, verify_signed_report};
use burd_protocol::{
    FULL_REPORT_TTL_SECONDS, ProviderHeartbeatSummary, ProviderSession, default_state_dir,
    evidence_freshness, heartbeat_summary_from_session, load_provider_session,
    redacted_config_value,
};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawData {
    pub redacted: bool,
    pub redacted_fields: Vec<String>,
    pub latest_report: Option<serde_json::Value>,
    pub latest_signed_report_summary: Option<serde_json::Value>,
    pub provider_details: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ProviderSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat: Option<ProviderHeartbeatSummary>,
    pub identity_redacted: Option<serde_json::Value>,
    pub config_redacted: Option<serde_json::Value>,
    pub history_summary: serde_json::Value,
    pub actions: serde_json::Value,
    pub logs_summary: serde_json::Value,
    pub verification: serde_json::Value,
    pub pricing: serde_json::Value,
    pub earnings_mock: serde_json::Value,
    pub uptime: serde_json::Value,
    pub reliability: serde_json::Value,
    pub network: serde_json::Value,
    pub ai_performance: serde_json::Value,
    pub capability_spot: serde_json::Value,
    pub workload_eligibility: serde_json::Value,
}

pub fn build_raw_data(agent_version: &str, host_uri: &str) -> RawData {
    let provider = build_provider_details(agent_version, host_uri);
    build_raw_data_from_provider(&provider, &provider.verification)
}

pub(crate) fn build_raw_data_from_provider(
    provider: &crate::provider::BurdProviderDetails,
    verification: &crate::verification::ProviderVerification,
) -> RawData {
    let config_redacted = redacted_config_value().ok();
    let heartbeat = heartbeat_summary_from_session(provider.session.as_ref());
    RawData {
        redacted: true,
        redacted_fields: vec![
            "private_key".to_string(),
            "secret_key_base64".to_string(),
            "private_key_path".to_string(),
            "api_token".to_string(),
            "api_token_hash".to_string(),
            "credentials".to_string(),
        ],
        latest_report: latest_report(),
        latest_signed_report_summary: signed_report_summary(),
        provider_details: serde_json::to_value(&provider)
            .unwrap_or_else(|_| serde_json::json!({"error": "provider serialization failed"})),
        session: load_provider_session().ok().flatten(),
        heartbeat,
        identity_redacted: config_redacted.clone(),
        config_redacted,
        history_summary: history_summary(),
        actions: serde_json::json!({
            "items": load_actions().unwrap_or_default(),
            "logs": load_logs().unwrap_or_default(),
        }),
        logs_summary: logs_summary().unwrap_or_else(|_| {
            serde_json::json!({
                "actions_total": 0,
                "logs_total": 0,
                "latest_action": null,
            })
        }),
        verification: serde_json::to_value(verification)
            .unwrap_or_else(|_| serde_json::json!({"error": "verification serialization failed"})),
        pricing: serde_json::to_value(&provider.pricing)
            .unwrap_or_else(|_| serde_json::json!({"error": "pricing serialization failed"})),
        earnings_mock: serde_json::to_value(&provider.estimated_earnings)
            .unwrap_or_else(|_| serde_json::json!({"error": "earnings serialization failed"})),
        uptime: serde_json::to_value(
            load_uptime_summary().unwrap_or_else(|_| provider.uptime.clone()),
        )
        .unwrap_or_else(|_| serde_json::json!({"error": "uptime serialization failed"})),
        reliability: serde_json::to_value(&provider.reliability)
            .unwrap_or_else(|_| serde_json::json!({"error": "reliability serialization failed"})),
        network: serde_json::to_value(&provider.network)
            .unwrap_or_else(|_| serde_json::json!({"error": "network serialization failed"})),
        ai_performance: serde_json::to_value(&provider.ai_performance).unwrap_or_else(
            |_| serde_json::json!({"error": "ai performance serialization failed"}),
        ),
        capability_spot: serde_json::to_value(&provider.capability_spot).unwrap_or_else(
            |_| serde_json::json!({"error": "capability spot serialization failed"}),
        ),
        workload_eligibility: serde_json::to_value(&provider.workload_eligibility).unwrap_or_else(
            |_| serde_json::json!({"error": "workload eligibility serialization failed"}),
        ),
    }
}

fn read_json(name: &str) -> Option<serde_json::Value> {
    let path = default_state_dir().join(name);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn signed_report_summary() -> Option<serde_json::Value> {
    let report = load_latest_signed_report().ok()?;
    let verification = verify_signed_report(&report);
    Some(serde_json::json!({
        "provider_id": report.provider_id,
        "machine_id": report.machine_id,
        "report_hash": report.report_hash,
        "key_algorithm": report.key_algorithm,
        "signed_at": report.signed_at,
        "evidence": verification.evidence,
        "signature_valid_locally": report.signature_valid_locally,
        "canonicalization_version": report.canonicalization_version,
    }))
}

fn latest_report() -> Option<serde_json::Value> {
    let mut report = read_json("latest-report.json")?;
    let timestamp = report.get("timestamp")?.as_str()?;
    let evidence = evidence_freshness(timestamp, FULL_REPORT_TTL_SECONDS).ok()?;
    report
        .as_object_mut()?
        .insert("evidence".to_string(), serde_json::to_value(evidence).ok()?);
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_data_serializes_without_private_key_label() {
        let raw = RawData {
            redacted: true,
            redacted_fields: vec!["private_key_path".to_string()],
            latest_report: None,
            latest_signed_report_summary: None,
            provider_details: serde_json::json!({}),
            session: None,
            heartbeat: None,
            identity_redacted: Some(serde_json::json!({"private_key_path": "[redacted]"})),
            config_redacted: Some(serde_json::json!({"private_key_path": "[redacted]"})),
            history_summary: serde_json::json!({}),
            actions: serde_json::json!({}),
            logs_summary: serde_json::json!({}),
            verification: serde_json::json!({}),
            pricing: serde_json::json!({}),
            earnings_mock: serde_json::json!({}),
            uptime: serde_json::json!({}),
            reliability: serde_json::json!({}),
            network: serde_json::json!({}),
            ai_performance: serde_json::json!({}),
            capability_spot: serde_json::json!({}),
            workload_eligibility: serde_json::json!({}),
        };
        let json = serde_json::to_string(&raw).unwrap();
        assert!(!json.contains("secret_key_base64"));
    }
}
