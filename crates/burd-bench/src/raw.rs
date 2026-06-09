use crate::actions::{load_actions, load_logs, logs_summary};
use crate::health::load_uptime_summary;
use crate::history::history_summary;
use crate::provider::build_provider_details;
use burd_protocol::{default_state_dir, redacted_config_value};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawData {
    pub redacted: bool,
    pub redacted_fields: Vec<String>,
    pub latest_report: Option<serde_json::Value>,
    pub latest_signed_report_summary: Option<serde_json::Value>,
    pub provider_details: serde_json::Value,
    pub identity_redacted: Option<serde_json::Value>,
    pub config_redacted: Option<serde_json::Value>,
    pub history_summary: serde_json::Value,
    pub actions: serde_json::Value,
    pub logs_summary: serde_json::Value,
    pub verification: serde_json::Value,
    pub pricing: serde_json::Value,
    pub earnings_mock: serde_json::Value,
    pub uptime: serde_json::Value,
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
        latest_report: read_json("latest-report.json"),
        latest_signed_report_summary: signed_report_summary(),
        provider_details: serde_json::to_value(&provider)
            .unwrap_or_else(|_| serde_json::json!({"error": "provider serialization failed"})),
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
    }
}

fn read_json(name: &str) -> Option<serde_json::Value> {
    let path = default_state_dir().join(name);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn signed_report_summary() -> Option<serde_json::Value> {
    let report = read_json("latest-signed-report.json")?;
    Some(serde_json::json!({
        "provider_id": report.get("provider_id"),
        "machine_id": report.get("machine_id"),
        "report_hash": report.get("report_hash"),
        "key_algorithm": report.get("key_algorithm"),
        "signed_at": report.get("signed_at"),
        "signature_valid_locally": report.get("signature_valid_locally"),
        "canonicalization_version": report.get("canonicalization_version"),
    }))
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
            identity_redacted: Some(serde_json::json!({"private_key_path": "[redacted]"})),
            config_redacted: Some(serde_json::json!({"private_key_path": "[redacted]"})),
            history_summary: serde_json::json!({}),
            actions: serde_json::json!({}),
            logs_summary: serde_json::json!({}),
            verification: serde_json::json!({}),
            pricing: serde_json::json!({}),
            earnings_mock: serde_json::json!({}),
            uptime: serde_json::json!({}),
        };
        let json = serde_json::to_string(&raw).unwrap();
        assert!(!json.contains("secret_key_base64"));
    }
}
