use crate::{default_state_dir, random_token};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteSessionStatus {
    PendingConnection,
    Online,
    Degraded,
    Offline,
    Expired,
    Revoked,
}

impl RemoteSessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingConnection => "pending_connection",
            Self::Online => "online",
            Self::Degraded => "degraded",
            Self::Offline => "offline",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
        }
    }

    pub fn terminal(self) -> bool {
        matches!(self, Self::Expired | Self::Revoked)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSessionResume {
    pub session_id: String,
    pub resume_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartRemoteSessionRequest {
    pub provider_id: String,
    pub device_id: String,
    pub hardware_fingerprint: String,
    pub agent_version: String,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_report_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_challenge_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<RemoteSessionResume>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartRemoteSessionResponse {
    pub request_id: String,
    pub session_id: String,
    pub resume_token: String,
    pub status: RemoteSessionStatus,
    pub expires_at: String,
    pub heartbeat_interval_seconds: u32,
    pub missed_heartbeat_limit: u32,
    pub sequence_start: u64,
    pub control_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSessionRecord {
    pub request_id: String,
    pub session_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub status: RemoteSessionStatus,
    pub sequence_last: u64,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disconnect_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientControlMessage {
    pub session_id: String,
    pub device_id: String,
    pub sequence: u64,
    pub sent_at: String,
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerControlMessage {
    pub request_id: String,
    pub session_id: String,
    pub sequence_ack: u64,
    pub server_time: String,
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    pub hardware_fingerprint: String,
    #[serde(default)]
    pub local_status: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatReceipt {
    pub request_id: String,
    pub session_id: String,
    pub sequence_ack: u64,
    pub status: RemoteSessionStatus,
    pub server_time: String,
    pub next_heartbeat_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSessionRevocationResponse {
    pub request_id: String,
    pub session_id: String,
    pub status: RemoteSessionStatus,
    pub revoked_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSessionState {
    pub control_plane_url: String,
    pub session_id: String,
    pub resume_token: String,
    pub expires_at: String,
    pub heartbeat_interval_seconds: u32,
    pub missed_heartbeat_limit: u32,
    pub sequence_last: u64,
    pub control_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSessionStateStatus {
    pub state_path: String,
    pub control_plane_url: String,
    pub session_id: String,
    pub expires_at: String,
    pub heartbeat_interval_seconds: u32,
    pub missed_heartbeat_limit: u32,
    pub sequence_last: u64,
    pub control_url: String,
    pub resume_token_present: bool,
}

pub fn remote_session_path() -> PathBuf {
    default_state_dir().join("remote-session.json")
}

pub fn load_remote_session() -> Result<RemoteSessionState, String> {
    let path = remote_session_path();
    let bytes =
        fs::read(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

pub fn save_remote_session(
    control_plane_url: &str,
    response: &StartRemoteSessionResponse,
) -> Result<RemoteSessionStateStatus, String> {
    let state = RemoteSessionState {
        control_plane_url: control_plane_url.trim_end_matches('/').to_string(),
        session_id: response.session_id.clone(),
        resume_token: response.resume_token.clone(),
        expires_at: response.expires_at.clone(),
        heartbeat_interval_seconds: response.heartbeat_interval_seconds,
        missed_heartbeat_limit: response.missed_heartbeat_limit,
        sequence_last: response.sequence_start,
        control_url: response.control_url.clone(),
    };
    write_remote_session(&state)?;
    Ok(show_remote_session_from(&state))
}

pub fn update_remote_session_sequence(sequence_last: u64) -> Result<(), String> {
    let mut state = load_remote_session()?;
    state.sequence_last = sequence_last;
    write_remote_session(&state)
}

pub fn show_remote_session() -> Result<RemoteSessionStateStatus, String> {
    load_remote_session().map(|state| show_remote_session_from(&state))
}

pub fn clear_remote_session() -> Result<(), String> {
    let path = remote_session_path();
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove {}: {error}", path.display())),
    }
}

fn show_remote_session_from(state: &RemoteSessionState) -> RemoteSessionStateStatus {
    RemoteSessionStateStatus {
        state_path: remote_session_path().display().to_string(),
        control_plane_url: state.control_plane_url.clone(),
        session_id: state.session_id.clone(),
        expires_at: state.expires_at.clone(),
        heartbeat_interval_seconds: state.heartbeat_interval_seconds,
        missed_heartbeat_limit: state.missed_heartbeat_limit,
        sequence_last: state.sequence_last,
        control_url: state.control_url.clone(),
        resume_token_present: !state.resume_token.is_empty(),
    }
}

fn write_remote_session(state: &RemoteSessionState) -> Result<(), String> {
    let path = remote_session_path();
    let parent = path
        .parent()
        .ok_or_else(|| "remote session state path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("failed to serialize remote session: {error}"))?;
    fs::write(&path, bytes).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn new_resume_token() -> Result<String, String> {
    random_token("session_resume")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_expired_and_revoked_are_terminal() {
        assert!(RemoteSessionStatus::Expired.terminal());
        assert!(RemoteSessionStatus::Revoked.terminal());
        assert!(!RemoteSessionStatus::Offline.terminal());
    }

    #[test]
    fn status_view_redacts_resume_token() {
        let state = RemoteSessionState {
            control_plane_url: "https://api.burd.cloud".to_string(),
            session_id: "session_test".to_string(),
            resume_token: "secret".to_string(),
            expires_at: "2026-01-01T00:00:00Z".to_string(),
            heartbeat_interval_seconds: 15,
            missed_heartbeat_limit: 3,
            sequence_last: 4,
            control_url: "wss://api.burd.cloud/v1/sessions/session_test/control".to_string(),
        };
        let value = serde_json::to_value(show_remote_session_from(&state)).unwrap();
        assert_eq!(value["resume_token_present"], true);
        assert!(value.get("resume_token").is_none());
        assert!(!value.to_string().contains("secret"));
    }
}
