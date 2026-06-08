use crate::actions::{load_actions, load_logs, logs_summary};
use crate::health::load_uptime_summary;
use crate::provider::build_provider_details;
use crate::verification::verify_provider;
use burd_protocol::{default_state_dir, redacted_config_value};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawData {
    pub latest_report: Option<serde_json::Value>,
    pub latest_signed_report: Option<serde_json::Value>,
    pub provider_details: serde_json::Value,
    pub config_redacted: Option<serde_json::Value>,
    pub actions: serde_json::Value,
    pub logs_summary: serde_json::Value,
    pub verification: serde_json::Value,
    pub pricing: serde_json::Value,
    pub earnings_mock: serde_json::Value,
    pub uptime: serde_json::Value,
}

pub fn build_raw_data(agent_version: &str, host_uri: &str) -> RawData {
    let provider = build_provider_details(agent_version, host_uri);
    RawData {
        latest_report: read_json("latest-report.json"),
        latest_signed_report: read_json("latest-signed-report.json"),
        provider_details: serde_json::to_value(&provider)
            .unwrap_or_else(|_| serde_json::json!({"error": "provider serialization failed"})),
        config_redacted: redacted_config_value().ok(),
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
        verification: serde_json::to_value(verify_provider(agent_version))
            .unwrap_or_else(|_| serde_json::json!({"error": "verification serialization failed"})),
        pricing: serde_json::to_value(&provider.pricing)
            .unwrap_or_else(|_| serde_json::json!({"error": "pricing serialization failed"})),
        earnings_mock: serde_json::to_value(&provider.estimated_earnings)
            .unwrap_or_else(|_| serde_json::json!({"error": "earnings serialization failed"})),
        uptime: serde_json::to_value(load_uptime_summary().unwrap_or(provider.uptime))
            .unwrap_or_else(|_| serde_json::json!({"error": "uptime serialization failed"})),
    }
}

fn read_json(name: &str) -> Option<serde_json::Value> {
    let path = default_state_dir().join(name);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_data_serializes_without_private_key_label() {
        let raw = RawData {
            latest_report: None,
            latest_signed_report: None,
            provider_details: serde_json::json!({}),
            config_redacted: Some(serde_json::json!({"private_key_path": "[redacted]"})),
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
