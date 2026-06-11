use crate::identity::default_state_dir;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSessionStatus {
    Inactive,
    Active,
    Expired,
    Invalidated,
    Stopped,
    Failed,
}

impl ProviderSessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inactive => "inactive",
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Invalidated => "invalidated",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSessionMode {
    MarketplaceLocal,
    LocalDiagnostic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderHeartbeatSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_error: Option<String>,
    pub heartbeat_count: u64,
    pub online_locally: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_matches_session: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSession {
    pub provider_session_id: String,
    pub provider_id: String,
    pub machine_id: String,
    pub hardware_fingerprint: String,
    pub started_at: String,
    pub last_heartbeat_at: String,
    pub status: ProviderSessionStatus,
    pub readiness_at_start: serde_json::Value,
    pub report_hash: String,
    pub challenge_id: String,
    pub expires_at: String,
    pub marketplace_policy_snapshot: serde_json::Value,
    pub evidence_summary: serde_json::Value,
    pub session_mode: ProviderSessionMode,
    pub online_locally: bool,
    pub is_expired: bool,
    pub heartbeat_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_fingerprint_matches_session: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_heartbeat_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSessionStatusReport {
    pub status: ProviderSessionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ProviderSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat: Option<ProviderHeartbeatSummary>,
    pub online_locally: bool,
    pub warnings: Vec<String>,
}

pub fn provider_session_path() -> PathBuf {
    default_state_dir().join("provider-session.json")
}

pub fn new_provider_session_id() -> String {
    format!("provider-session-{}", Uuid::new_v4())
}

pub fn save_provider_session(session: &ProviderSession) -> Result<(), String> {
    let path = provider_session_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(session)
        .map_err(|error| format!("failed to serialize provider session: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn load_provider_session() -> Result<Option<ProviderSession>, String> {
    let path = provider_session_path();
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("failed to read {}: {error}", path.display()));
        }
    };
    let session: ProviderSession = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid provider session JSON: {error}"))?;
    Ok(Some(session))
}

pub fn session_status_from_session(
    mut session: ProviderSession,
    status: ProviderSessionStatus,
    online_locally: bool,
) -> ProviderSession {
    session.status = status;
    session.online_locally = online_locally;
    session.is_expired = matches!(status, ProviderSessionStatus::Expired);
    session
}

pub fn heartbeat_summary_from_session(
    session: Option<&ProviderSession>,
) -> Option<ProviderHeartbeatSummary> {
    let session = session?;
    if session.heartbeat_count == 0
        && session.last_heartbeat_status.is_none()
        && session.last_heartbeat_error.is_none()
    {
        return None;
    }
    Some(ProviderHeartbeatSummary {
        last_heartbeat_at: Some(session.last_heartbeat_at.clone()),
        last_heartbeat_status: session.last_heartbeat_status.clone(),
        last_heartbeat_error: session.last_heartbeat_error.clone(),
        heartbeat_count: session.heartbeat_count,
        online_locally: session.online_locally,
        fingerprint_matches_session: session.last_heartbeat_fingerprint_matches_session,
        warnings: session.last_heartbeat_warnings.clone(),
    })
}

pub fn active_provider_session(
    provider_id: String,
    machine_id: String,
    hardware_fingerprint: String,
    readiness_at_start: serde_json::Value,
    report_hash: String,
    challenge_id: String,
    expires_at: String,
    marketplace_policy_snapshot: serde_json::Value,
    evidence_summary: serde_json::Value,
    session_mode: ProviderSessionMode,
    warnings: Vec<String>,
) -> ProviderSession {
    let started_at = Utc::now().to_rfc3339();
    ProviderSession {
        provider_session_id: new_provider_session_id(),
        provider_id,
        machine_id,
        hardware_fingerprint,
        started_at: started_at.clone(),
        last_heartbeat_at: started_at,
        status: ProviderSessionStatus::Active,
        readiness_at_start,
        report_hash,
        challenge_id,
        expires_at,
        marketplace_policy_snapshot,
        evidence_summary,
        session_mode,
        online_locally: true,
        is_expired: false,
        heartbeat_count: 0,
        last_heartbeat_status: None,
        last_heartbeat_error: None,
        last_heartbeat_fingerprint_matches_session: None,
        last_heartbeat_warnings: Vec::new(),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_roundtrip_serializes() {
        let session = active_provider_session(
            "provider".to_string(),
            "machine".to_string(),
            "sha256:test".to_string(),
            serde_json::json!({"status":"ready_locally"}),
            "hash".to_string(),
            "challenge".to_string(),
            "2099-01-01T00:00:00Z".to_string(),
            serde_json::json!({"marketplace_eligible": true}),
            serde_json::json!({"signed_report":{"is_expired":false}}),
            ProviderSessionMode::MarketplaceLocal,
            vec![],
        );
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("provider_session_id"));
        assert!(json.contains("marketplace_policy_snapshot"));
    }
}
