use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::enrollment::{DeviceAuth, EnrollmentError, authenticate_device};
use burd_protocol::{
    ClientControlMessage, HeartbeatPayload, HeartbeatReceipt, RemoteSessionRecord,
    RemoteSessionRevocationResponse, RemoteSessionStatus, StartRemoteSessionRequest,
    StartRemoteSessionResponse, hash_canonical, new_resume_token, sha256_hex,
};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

#[derive(Debug)]
pub enum SessionError {
    Database(DbError),
    NotFound(String),
    Invalid(String),
    Unauthorized,
    Expired,
    Revoked,
    Conflict(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "{error}"),
            Self::NotFound(message) | Self::Invalid(message) | Self::Conflict(message) => {
                formatter.write_str(message)
            }
            Self::Unauthorized => formatter.write_str("session credential is invalid"),
            Self::Expired => formatter.write_str("session has expired"),
            Self::Revoked => formatter.write_str("session has been revoked"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<DbError> for SessionError {
    fn from(error: DbError) -> Self {
        Self::Database(error)
    }
}

impl From<tokio_postgres::Error> for SessionError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Database(DbError::new(error.to_string()))
    }
}

impl From<EnrollmentError> for SessionError {
    fn from(error: EnrollmentError) -> Self {
        match error {
            EnrollmentError::Database(error) => Self::Database(error),
            EnrollmentError::Revoked => Self::Revoked,
            EnrollmentError::Expired => Self::Expired,
            EnrollmentError::Conflict(message) => Self::Conflict(message),
            EnrollmentError::Invalid(message) => Self::Invalid(message),
            EnrollmentError::NotFound(message) => Self::NotFound(message),
            _ => Self::Unauthorized,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthorizedSession {
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub sequence_last: u64,
    pub heartbeat_interval_seconds: u32,
    pub missed_heartbeat_limit: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct RemoteSessionPolicy {
    pub ttl_seconds: u32,
    pub heartbeat_interval_seconds: u32,
    pub missed_heartbeat_limit: u32,
}
#[derive(Debug, Clone)]
pub struct ControlChannelLease {
    pub connection_id: String,
    pub revocation: watch::Receiver<Option<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct ControlChannelRegistry {
    channels: Arc<Mutex<HashMap<String, ActiveChannel>>>,
}

#[derive(Debug, Clone)]
struct ActiveChannel {
    connection_id: String,
    revocation: watch::Sender<Option<String>>,
}

impl ControlChannelRegistry {
    pub fn register(&self, session_id: &str) -> Result<ControlChannelLease, SessionError> {
        let mut channels = self
            .channels
            .lock()
            .expect("control channel mutex poisoned");
        if channels.contains_key(session_id) {
            return Err(SessionError::Conflict(
                "a control channel is already connected for this session".to_string(),
            ));
        }
        let connection_id = format!("connection_{}", Uuid::new_v4());
        let (sender, receiver) = watch::channel(None);
        channels.insert(
            session_id.to_string(),
            ActiveChannel {
                connection_id: connection_id.clone(),
                revocation: sender,
            },
        );
        Ok(ControlChannelLease {
            connection_id,
            revocation: receiver,
        })
    }

    pub fn revoke(&self, session_id: &str, reason: &str) {
        let channels = self
            .channels
            .lock()
            .expect("control channel mutex poisoned");
        if let Some(channel) = channels.get(session_id) {
            let _ = channel.revocation.send(Some(reason.to_string()));
        }
    }

    pub fn release(&self, session_id: &str, connection_id: &str) {
        let mut channels = self
            .channels
            .lock()
            .expect("control channel mutex poisoned");
        let should_remove = channels
            .get(session_id)
            .is_some_and(|channel| channel.connection_id == connection_id);
        if should_remove {
            channels.remove(session_id);
        }
    }
}

impl Database {
    pub async fn start_remote_session(
        &self,
        request_id: &str,
        credential: &str,
        request: &StartRemoteSessionRequest,
        policy: RemoteSessionPolicy,
        control_url: String,
    ) -> Result<StartRemoteSessionResponse, SessionError> {
        if request.hardware_fingerprint.trim().is_empty() || request.agent_version.trim().is_empty()
        {
            return Err(SessionError::Invalid(
                "hardware_fingerprint and agent_version are required".to_string(),
            ));
        }
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let auth = authenticate_device(&transaction, &request.device_id, credential).await?;
        validate_claimed_identity(&auth, request)?;
        let enrolled_fingerprint = transaction
            .query_opt(
                "SELECT hardware_fingerprint FROM device_enrollments WHERE device_id = $1 AND status = 'completed' ORDER BY completed_at DESC LIMIT 1",
                &[&auth.device_id],
            )
            .await?
            .map(|row| row.get::<_, String>("hardware_fingerprint"));
        if enrolled_fingerprint
            .as_deref()
            .is_some_and(|fingerprint| fingerprint != request.hardware_fingerprint)
        {
            return Err(SessionError::Conflict(
                "hardware fingerprint changed since enrollment; re-verification is required"
                    .to_string(),
            ));
        }
        expire_stale_in_transaction(&transaction).await?;

        let now = Utc::now();
        let expires_at = (now + Duration::seconds(i64::from(policy.ttl_seconds))).to_rfc3339();
        let capabilities_json = serde_json::to_string(&request.capabilities)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;

        let mut response = if let Some(resume) = &request.resume {
            let resume_hash = sha256_hex(resume.resume_token.as_bytes());
            let row = transaction
                .query_opt(
                    "SELECT session_id, provider_id, device_id, status, sequence_last, resume_token_hash, expires_at FROM provider_sessions WHERE session_id = $1 FOR UPDATE",
                    &[&resume.session_id],
                )
                .await?
                .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
            authorize_session_row(&row, &auth, &resume_hash)?;
            let status: String = row.get("status");
            if status == "online" {
                return Err(SessionError::Conflict(
                    "session is already connected".to_string(),
                ));
            }
            if status == "revoked" {
                return Err(SessionError::Revoked);
            }
            if status == "expired" || timestamp_expired(row.get("expires_at"))? {
                return Err(SessionError::Expired);
            }
            let sequence_last = row.get::<_, i64>("sequence_last").max(0) as u64;
            transaction
                .execute(
                    "UPDATE provider_sessions SET status = 'pending_connection', hardware_fingerprint = $1, agent_version = $2, capabilities_json = $3, latest_report_hash = $4, latest_challenge_id = $5, expires_at = $6, disconnect_reason = NULL, updated_at = $7 WHERE session_id = $8",
                    &[&request.hardware_fingerprint, &request.agent_version, &capabilities_json, &request.latest_report_hash, &request.latest_challenge_id, &expires_at, &now.to_rfc3339(), &resume.session_id],
                )
                .await?;
            StartRemoteSessionResponse {
                request_id: request_id.to_string(),
                session_id: resume.session_id.clone(),
                resume_token: resume.resume_token.clone(),
                status: RemoteSessionStatus::PendingConnection,
                expires_at,
                heartbeat_interval_seconds: policy.heartbeat_interval_seconds,
                missed_heartbeat_limit: policy.missed_heartbeat_limit,
                sequence_start: sequence_last,
                control_url,
            }
        } else {
            let active = transaction
                .query_opt(
                    "SELECT session_id FROM provider_sessions WHERE device_id = $1 AND status IN ('pending_connection', 'online', 'degraded', 'offline') FOR UPDATE",
                    &[&auth.device_id],
                )
                .await?;
            if active.is_some() {
                return Err(SessionError::Conflict(
                    "device already has an active remote session; resume it".to_string(),
                ));
            }
            let session_id = format!("session_{}", Uuid::new_v4());
            let resume_token = new_resume_token().map_err(SessionError::Invalid)?;
            let resume_token_hash = sha256_hex(resume_token.as_bytes());
            let started_at = now.to_rfc3339();
            transaction
                .execute(
                    "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, resume_token_hash, hardware_fingerprint, agent_version, capabilities_json, latest_report_hash, latest_challenge_id, heartbeat_interval_seconds, missed_heartbeat_limit, updated_at) VALUES ($1, $2, $3, 'pending_connection', 0, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $4)",
                    &[&session_id, &auth.provider_id, &auth.device_id, &started_at, &expires_at, &resume_token_hash, &request.hardware_fingerprint, &request.agent_version, &capabilities_json, &request.latest_report_hash, &request.latest_challenge_id, &(policy.heartbeat_interval_seconds as i32), &(policy.missed_heartbeat_limit as i32)],
                )
                .await?;
            StartRemoteSessionResponse {
                request_id: request_id.to_string(),
                session_id,
                resume_token,
                status: RemoteSessionStatus::PendingConnection,
                expires_at,
                heartbeat_interval_seconds: policy.heartbeat_interval_seconds,
                missed_heartbeat_limit: policy.missed_heartbeat_limit,
                sequence_start: 0,
                control_url,
            }
        };

        response.control_url = response
            .control_url
            .replace("{session_id}", &response.session_id);
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device",
                actor_id: Some(auth.device_id),
                entity_type: "provider_session",
                entity_id: &response.session_id,
                event_type: if request.resume.is_some() {
                    "provider_session.resumed"
                } else {
                    "provider_session.started"
                },
                idempotency_key: None,
                summary: "remote provider session authorized",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(response)
    }

    pub async fn authorize_remote_session(
        &self,
        session_id: &str,
        device_id: &str,
        credential: &str,
        resume_token: &str,
        allow_terminal: bool,
    ) -> Result<AuthorizedSession, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let auth = authenticate_device(&transaction, device_id, credential).await?;
        let row = transaction
            .query_opt(
                "SELECT session_id, provider_id, device_id, status, sequence_last, resume_token_hash, expires_at, heartbeat_interval_seconds, missed_heartbeat_limit FROM provider_sessions WHERE session_id = $1 FOR UPDATE",
                &[&session_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
        authorize_session_row(&row, &auth, &sha256_hex(resume_token.as_bytes()))?;
        let status: String = row.get("status");
        if status == "revoked" && !allow_terminal {
            return Err(SessionError::Revoked);
        }
        if status == "expired" || timestamp_expired(row.get("expires_at"))? {
            transaction
                .execute(
                    "UPDATE provider_sessions SET status = 'expired', updated_at = $1 WHERE session_id = $2",
                    &[&Utc::now().to_rfc3339(), &session_id],
                )
                .await?;
            if !allow_terminal {
                transaction.commit().await?;
                return Err(SessionError::Expired);
            }
        }
        let authorized = AuthorizedSession {
            provider_id: auth.provider_id,
            device_id: auth.device_id,
            session_id: session_id.to_string(),
            sequence_last: row.get::<_, i64>("sequence_last").max(0) as u64,
            heartbeat_interval_seconds: row.get::<_, i32>("heartbeat_interval_seconds") as u32,
            missed_heartbeat_limit: row.get::<_, i32>("missed_heartbeat_limit") as u32,
        };
        transaction.commit().await?;
        Ok(authorized)
    }

    pub async fn mark_remote_session_connected(
        &self,
        session_id: &str,
        connection_id: &str,
    ) -> Result<(), SessionError> {
        let client = self.connect().await?;
        let now = Utc::now().to_rfc3339();
        let changed = client
            .execute(
                "UPDATE provider_sessions SET status = 'online', connection_id = $1, connected_at = COALESCE(connected_at, $2), disconnected_at = NULL, disconnect_reason = NULL, updated_at = $2 WHERE session_id = $3 AND status IN ('pending_connection', 'offline', 'degraded')",
                &[&connection_id, &now, &session_id],
            )
            .await?;
        if changed == 0 {
            return Err(SessionError::Conflict(
                "remote session is not connectable".to_string(),
            ));
        }
        Ok(())
    }

    pub async fn record_remote_heartbeat(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        message: &ClientControlMessage,
        session_ttl_seconds: u32,
    ) -> Result<HeartbeatReceipt, SessionError> {
        if message.session_id != authorized.session_id || message.device_id != authorized.device_id
        {
            return Err(SessionError::Unauthorized);
        }
        if message.message_type != "heartbeat" {
            return Err(SessionError::Invalid(
                "only heartbeat messages are accepted in BN-03".to_string(),
            ));
        }
        if message.sequence > i64::MAX as u64 {
            return Err(SessionError::Invalid(
                "heartbeat sequence exceeds the supported range".to_string(),
            ));
        }
        let payload: HeartbeatPayload =
            serde_json::from_value(message.payload.clone()).map_err(|error| {
                SessionError::Invalid(format!("invalid heartbeat payload: {error}"))
            })?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_one(
                "SELECT status, sequence_last, hardware_fingerprint, expires_at FROM provider_sessions WHERE session_id = $1 FOR UPDATE",
                &[&authorized.session_id],
            )
            .await?;
        let current_status: String = row.get("status");
        if current_status == "revoked" {
            return Err(SessionError::Revoked);
        }
        if current_status == "expired" || timestamp_expired(row.get("expires_at"))? {
            return Err(SessionError::Expired);
        }
        let sequence_last = row.get::<_, i64>("sequence_last").max(0) as u64;
        if message.sequence <= sequence_last {
            let conflict = format!(
                "heartbeat sequence {} was already observed; last sequence is {sequence_last}",
                message.sequence
            );
            let metadata = serde_json::json!({
                "received_sequence": message.sequence,
                "sequence_last": sequence_last,
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "device",
                    actor_id: Some(authorized.device_id.clone()),
                    entity_type: "provider_session",
                    entity_id: &authorized.session_id,
                    event_type: "provider_session.sequence_rejected",
                    idempotency_key: None,
                    summary: "duplicate or stale heartbeat sequence rejected",
                    metadata_json: &metadata,
                },
            )
            .await?;
            transaction.commit().await?;
            return Err(SessionError::Conflict(conflict));
        }
        let sequence_gap = message.sequence.saturating_sub(sequence_last + 1);
        let expected_fingerprint: Option<String> = row.get("hardware_fingerprint");
        let fingerprint_matches =
            expected_fingerprint.as_deref() == Some(&payload.hardware_fingerprint);
        let degraded = sequence_gap > 0 || !fingerprint_matches;
        let status = if degraded {
            RemoteSessionStatus::Degraded
        } else {
            RemoteSessionStatus::Online
        };
        let now = Utc::now();
        let server_time = now.to_rfc3339();
        let expires_at = (now + Duration::seconds(i64::from(session_ttl_seconds))).to_rfc3339();
        let payload_json = serde_json::to_string(&message.payload)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let payload_hash = hash_canonical(&message.payload)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let heartbeat_id = format!("heartbeat_{}", Uuid::new_v4());
        transaction
            .execute(
                "INSERT INTO session_heartbeats (heartbeat_id, session_id, sequence, client_sent_at, server_received_at, sequence_gap, payload_hash, payload_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&heartbeat_id, &authorized.session_id, &(message.sequence as i64), &message.sent_at, &server_time, &(sequence_gap as i64), &payload_hash, &payload_json],
            )
            .await?;
        transaction
            .execute(
                "UPDATE provider_sessions SET status = $1, sequence_last = $2, last_seen_at = $3, expires_at = $4, degraded_at = CASE WHEN $1 = 'degraded' THEN $3 ELSE degraded_at END, updated_at = $3 WHERE session_id = $5",
                &[&status.as_str(), &(message.sequence as i64), &server_time, &expires_at, &authorized.session_id],
            )
            .await?;
        if degraded {
            let metadata = serde_json::json!({
                "sequence_gap": sequence_gap,
                "fingerprint_matches": fingerprint_matches,
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "device",
                    actor_id: Some(authorized.device_id.clone()),
                    entity_type: "provider_session",
                    entity_id: &authorized.session_id,
                    event_type: "provider_session.degraded",
                    idempotency_key: None,
                    summary: "remote session heartbeat degraded",
                    metadata_json: &metadata,
                },
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(HeartbeatReceipt {
            request_id: request_id.to_string(),
            session_id: authorized.session_id.clone(),
            sequence_ack: message.sequence,
            status,
            server_time,
            next_heartbeat_seconds: authorized.heartbeat_interval_seconds,
        })
    }

    pub async fn mark_remote_session_disconnected(
        &self,
        session_id: &str,
        connection_id: &str,
        reason: &str,
    ) -> Result<(), SessionError> {
        let client = self.connect().await?;
        let now = Utc::now().to_rfc3339();
        client
            .execute(
                "UPDATE provider_sessions SET status = 'offline', connection_id = NULL, disconnected_at = $1, disconnect_reason = $2, updated_at = $1 WHERE session_id = $3 AND connection_id = $4 AND status NOT IN ('expired', 'revoked')",
                &[&now, &reason, &session_id, &connection_id],
            )
            .await?;
        Ok(())
    }

    pub async fn get_remote_session(
        &self,
        session_id: &str,
        authorized: &AuthorizedSession,
        request_id: &str,
    ) -> Result<RemoteSessionRecord, SessionError> {
        if session_id != authorized.session_id {
            return Err(SessionError::Unauthorized);
        }
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT session_id, provider_id, device_id, status, sequence_last, started_at, connected_at, last_seen_at, expires_at, disconnect_reason FROM provider_sessions WHERE session_id = $1",
                &[&session_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
        remote_session_from_row(row, request_id)
    }

    pub async fn revoke_remote_session(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<RemoteSessionRevocationResponse, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        let row = transaction
            .query_opt(
                "UPDATE provider_sessions SET status = 'revoked', revoked_at = $1, connection_id = NULL, disconnect_reason = 'revoked_by_admin', updated_at = $1 WHERE session_id = $2 RETURNING provider_id",
                &[&now, &session_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
        let provider_id: String = row.get("provider_id");
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "provider_session",
                entity_id: session_id,
                event_type: "provider_session.revoked",
                idempotency_key: None,
                summary: "remote provider session revoked",
                metadata_json: &serde_json::json!({ "provider_id": provider_id }).to_string(),
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(RemoteSessionRevocationResponse {
            request_id: request_id.to_string(),
            session_id: session_id.to_string(),
            status: RemoteSessionStatus::Revoked,
            revoked_at: now,
        })
    }

    pub async fn expire_stale_remote_sessions(&self) -> Result<u64, SessionError> {
        let client = self.connect().await?;
        let now = Utc::now().to_rfc3339();
        Ok(client
            .execute(
                "UPDATE provider_sessions SET status = 'expired', connection_id = NULL, disconnect_reason = 'server_ttl_elapsed', updated_at = $1 WHERE status IN ('pending_connection', 'online', 'degraded', 'offline') AND expires_at <= $1",
                &[&now],
            )
            .await?)
    }
}

fn validate_claimed_identity(
    auth: &DeviceAuth,
    request: &StartRemoteSessionRequest,
) -> Result<(), SessionError> {
    if auth.provider_id != request.provider_id || auth.device_id != request.device_id {
        return Err(SessionError::Unauthorized);
    }
    Ok(())
}

fn authorize_session_row(
    row: &Row,
    auth: &DeviceAuth,
    resume_hash: &str,
) -> Result<(), SessionError> {
    let stored_hash: Option<String> = row.get("resume_token_hash");
    let provider_id: String = row.get("provider_id");
    let device_id: Option<String> = row.get("device_id");
    if provider_id != auth.provider_id
        || device_id.as_deref() != Some(&auth.device_id)
        || !stored_hash
            .as_deref()
            .is_some_and(|stored| constant_time_eq(stored.as_bytes(), resume_hash.as_bytes()))
    {
        return Err(SessionError::Unauthorized);
    }
    Ok(())
}

async fn expire_stale_in_transaction(transaction: &Transaction<'_>) -> Result<(), SessionError> {
    let now = Utc::now().to_rfc3339();
    transaction
        .execute(
            "UPDATE provider_sessions SET status = 'expired', connection_id = NULL, disconnect_reason = 'server_ttl_elapsed', updated_at = $1 WHERE status IN ('pending_connection', 'online', 'degraded', 'offline') AND expires_at <= $1",
            &[&now],
        )
        .await?;
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
fn timestamp_expired(raw: String) -> Result<bool, SessionError> {
    let timestamp = chrono::DateTime::parse_from_rfc3339(&raw)
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    Ok(timestamp <= Utc::now())
}

fn remote_session_from_row(
    row: Row,
    request_id: &str,
) -> Result<RemoteSessionRecord, SessionError> {
    let status: String = row.get("status");
    Ok(RemoteSessionRecord {
        request_id: request_id.to_string(),
        session_id: row.get("session_id"),
        provider_id: row.get("provider_id"),
        device_id: row
            .get::<_, Option<String>>("device_id")
            .ok_or_else(|| SessionError::Invalid("remote session has no device".to_string()))?,
        status: parse_status(&status)?,
        sequence_last: row.get::<_, i64>("sequence_last").max(0) as u64,
        started_at: row.get("started_at"),
        connected_at: row.get("connected_at"),
        last_seen_at: row.get("last_seen_at"),
        expires_at: row.get("expires_at"),
        disconnect_reason: row.get("disconnect_reason"),
    })
}

fn parse_status(status: &str) -> Result<RemoteSessionStatus, SessionError> {
    match status {
        "pending_connection" => Ok(RemoteSessionStatus::PendingConnection),
        "online" => Ok(RemoteSessionStatus::Online),
        "degraded" => Ok(RemoteSessionStatus::Degraded),
        "offline" => Ok(RemoteSessionStatus::Offline),
        "expired" => Ok(RemoteSessionStatus::Expired),
        "revoked" => Ok(RemoteSessionStatus::Revoked),
        _ => Err(SessionError::Invalid(format!(
            "unknown remote session status {status}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rejects_duplicate_control_channels_and_releases_by_owner() {
        let registry = ControlChannelRegistry::default();
        let first = registry.register("session_1").unwrap();
        assert!(registry.register("session_1").is_err());
        registry.release("session_1", "different");
        assert!(registry.register("session_1").is_err());
        registry.release("session_1", &first.connection_id);
        assert!(registry.register("session_1").is_ok());
    }

    #[test]
    fn registry_delivers_revocation_reason() {
        let registry = ControlChannelRegistry::default();
        let lease = registry.register("session_1").unwrap();
        registry.revoke("session_1", "blocked");
        assert_eq!(lease.revocation.borrow().as_deref(), Some("blocked"));
    }

    #[tokio::test]
    #[ignore]
    async fn remote_session_lifecycle_is_authoritative_in_postgres() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_session_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let now = Utc::now().to_rfc3339();
        let credential = "burd_device_test_secret";
        let credential_hash = sha256_hex(credential.as_bytes());
        let credential_expires = (Utc::now() + Duration::minutes(30)).to_rfc3339();
        let client = db.connect().await.unwrap();
        client.execute(
            "INSERT INTO providers (provider_id, status, created_at, updated_at) VALUES ('provider_test', 'enrolled', $1, $1)",
            &[&now],
        ).await.unwrap();
        client.execute(
            "INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ('device_test', 'provider_test', 'machine_test', 'active', $1, $1)",
            &[&now],
        ).await.unwrap();
        client.execute(
            "INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at) VALUES ('key_test', 'provider_test', 'device_test', 'public_test', 'ed25519', 'active', $1)",
            &[&now],
        ).await.unwrap();
        client.execute(
            "INSERT INTO device_credentials (credential_id, provider_id, device_id, credential_hash, status, issued_at, expires_at) VALUES ('credential_test', 'provider_test', 'device_test', $1, 'active', $2, $3)",
            &[&credential_hash, &now, &credential_expires],
        ).await.unwrap();

        let request = StartRemoteSessionRequest {
            provider_id: "provider_test".to_string(),
            device_id: "device_test".to_string(),
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            agent_version: "0.1.0".to_string(),
            capabilities: serde_json::json!({"backend": "cuda"}),
            latest_report_hash: None,
            latest_challenge_id: None,
            resume: None,
        };
        let started = db
            .start_remote_session(
                "req_start",
                credential,
                &request,
                RemoteSessionPolicy {
                    ttl_seconds: 900,
                    heartbeat_interval_seconds: 15,
                    missed_heartbeat_limit: 3,
                },
                "ws://localhost/v1/sessions/{session_id}/control".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(started.status, RemoteSessionStatus::PendingConnection);
        assert!(started.control_url.contains(&started.session_id));
        assert!(matches!(
            db.start_remote_session(
                "req_duplicate",
                credential,
                &request,
                RemoteSessionPolicy {
                    ttl_seconds: 900,
                    heartbeat_interval_seconds: 15,
                    missed_heartbeat_limit: 3,
                },
                "ws://localhost/v1/sessions/{session_id}/control".to_string(),
            )
            .await,
            Err(SessionError::Conflict(_))
        ));

        let authorized = db
            .authorize_remote_session(
                &started.session_id,
                "device_test",
                credential,
                &started.resume_token,
                false,
            )
            .await
            .unwrap();
        db.mark_remote_session_connected(&started.session_id, "connection_test")
            .await
            .unwrap();
        let heartbeat = |sequence| ClientControlMessage {
            session_id: started.session_id.clone(),
            device_id: "device_test".to_string(),
            sequence,
            sent_at: Utc::now().to_rfc3339(),
            message_type: "heartbeat".to_string(),
            payload: serde_json::json!({
                "hardware_fingerprint": "sha256:fingerprint",
                "local_status": {}
            }),
        };
        let first = db
            .record_remote_heartbeat("req_hb_1", &authorized, &heartbeat(1), 900)
            .await
            .unwrap();
        assert_eq!(first.status, RemoteSessionStatus::Online);
        let gap = db
            .record_remote_heartbeat("req_hb_3", &authorized, &heartbeat(3), 900)
            .await
            .unwrap();
        assert_eq!(gap.status, RemoteSessionStatus::Degraded);
        assert!(matches!(
            db.record_remote_heartbeat("req_replay", &authorized, &heartbeat(3), 900)
                .await,
            Err(SessionError::Conflict(_))
        ));

        db.mark_remote_session_disconnected(
            &started.session_id,
            "connection_test",
            "test_disconnect",
        )
        .await
        .unwrap();
        let resumed = db
            .start_remote_session(
                "req_resume",
                credential,
                &StartRemoteSessionRequest {
                    resume: Some(burd_protocol::RemoteSessionResume {
                        session_id: started.session_id.clone(),
                        resume_token: started.resume_token.clone(),
                    }),
                    ..request
                },
                RemoteSessionPolicy {
                    ttl_seconds: 900,
                    heartbeat_interval_seconds: 15,
                    missed_heartbeat_limit: 3,
                },
                "ws://localhost/v1/sessions/{session_id}/control".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(resumed.sequence_start, 3);
        let revoked = db
            .revoke_remote_session(&started.session_id, "req_revoke")
            .await
            .unwrap();
        assert_eq!(revoked.status, RemoteSessionStatus::Revoked);
        assert!(matches!(
            db.authorize_remote_session(
                &started.session_id,
                "device_test",
                credential,
                &started.resume_token,
                false,
            )
            .await,
            Err(SessionError::Revoked)
        ));

        db.drop_schema_for_test().await.unwrap();
    }
}
