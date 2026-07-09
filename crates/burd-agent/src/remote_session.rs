use crate::remote_enrollment::{join_url, post_json, refresh_credential};
use burd_bench::build_registration_payload;
use burd_protocol::{
    ClientControlMessage, HeartbeatPayload, RemoteEnrollmentState, RemoteSessionRecord,
    RemoteSessionResume, RemoteSessionState, RemoteSessionStateStatus, ServerControlMessage,
    StartRemoteSessionRequest, StartRemoteSessionResponse, clear_remote_session,
    load_remote_enrollment, load_remote_session, save_remote_session, show_remote_session,
    update_remote_session_sequence,
};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};

pub fn connect(
    agent_version: &str,
    max_reconnect_delay_seconds: u64,
) -> Result<RemoteSessionStateStatus, String> {
    ensure_credential_fresh()?;
    start_or_resume(agent_version)?;
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| format!("failed to start remote session runtime: {error}"))?;
    runtime.block_on(run_control_loop(
        agent_version.to_string(),
        max_reconnect_delay_seconds.max(1),
    ))?;
    show_remote_session()
}

pub fn status() -> Result<RemoteSessionRecord, String> {
    let enrollment = load_remote_enrollment()?;
    let session = load_remote_session()?;
    let url = join_url(
        &session.control_plane_url,
        &format!("/v1/sessions/{}", session.session_id),
    );
    let request = ureq::get(&url)
        .header(
            "Authorization",
            &format!("Bearer {}", enrollment.credential),
        )
        .header("X-Burd-Session-Token", &session.resume_token)
        .header("X-Burd-Device-Id", &enrollment.device_id)
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build();
    let mut response = request
        .call()
        .map_err(|error| format!("control plane request failed: {error}"))?;
    let status = response.status();
    let value: serde_json::Value = response
        .body_mut()
        .read_json()
        .map_err(|error| format!("control plane returned invalid JSON: {error}"))?;
    if !status.is_success() {
        return Err(remote_error(&value));
    }
    serde_json::from_value(value)
        .map_err(|error| format!("invalid remote session response contract: {error}"))
}

fn start_or_resume(agent_version: &str) -> Result<RemoteSessionStateStatus, String> {
    let enrollment = load_remote_enrollment()?;
    let registration = build_registration_payload(agent_version);
    let persisted = load_remote_session().ok();
    let request = StartRemoteSessionRequest {
        provider_id: enrollment.provider_id.clone(),
        device_id: enrollment.device_id.clone(),
        hardware_fingerprint: registration.hardware_fingerprint,
        agent_version: agent_version.to_string(),
        capabilities: registration.capabilities,
        latest_report_hash: registration.latest_signed_report_hash,
        latest_challenge_id: None,
        resume: persisted.as_ref().map(|state| RemoteSessionResume {
            session_id: state.session_id.clone(),
            resume_token: state.resume_token.clone(),
        }),
    };
    let result: Result<StartRemoteSessionResponse, String> = post_json(
        &join_url(&enrollment.control_plane_url, "/v1/sessions"),
        &request,
        Some(&enrollment.credential),
    );
    let response = match result {
        Ok(response) => response,
        Err(error)
            if persisted.is_some()
                && (error.contains("expired") || error.contains("not_found")) =>
        {
            clear_remote_session()?;
            return start_or_resume(agent_version);
        }
        Err(error) => return Err(error),
    };
    save_remote_session(&enrollment.control_plane_url, &response)
}

async fn run_control_loop(
    agent_version: String,
    max_reconnect_delay_seconds: u64,
) -> Result<(), String> {
    let mut backoff = 1_u64;
    loop {
        let enrollment = ensure_credential_fresh()?;
        let session = load_remote_session()?;
        match run_one_connection(enrollment, session).await {
            Ok(ConnectionOutcome::Stopped) => return Ok(()),
            Ok(ConnectionOutcome::Terminal(reason)) => return Err(reason),
            Err(error) => {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "remote_session_reconnect",
                        "delay_seconds": backoff,
                        "error": error,
                    })
                );
            }
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
        }
        match start_or_resume(&agent_version) {
            Ok(_) => backoff = 1,
            Err(error) => {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "remote_session_resume_failed",
                        "error": error,
                    })
                );
                backoff = (backoff.saturating_mul(2)).min(max_reconnect_delay_seconds);
            }
        }
    }
}

enum ConnectionOutcome {
    Stopped,
    Terminal(String),
}

async fn run_one_connection(
    mut enrollment: RemoteEnrollmentState,
    session: RemoteSessionState,
) -> Result<ConnectionOutcome, String> {
    let mut request = session
        .control_url
        .clone()
        .into_client_request()
        .map_err(|error| format!("invalid control channel URL: {error}"))?;
    let headers = request.headers_mut();
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_str(&format!("Bearer {}", enrollment.credential))
            .map_err(|error| format!("invalid device credential header: {error}"))?,
    );
    headers.insert(
        HeaderName::from_static("x-burd-session-token"),
        HeaderValue::from_str(&session.resume_token)
            .map_err(|error| format!("invalid session token header: {error}"))?,
    );
    headers.insert(
        HeaderName::from_static("x-burd-device-id"),
        HeaderValue::from_str(&enrollment.device_id)
            .map_err(|error| format!("invalid device ID header: {error}"))?,
    );
    let (mut socket, _) = connect_async(request)
        .await
        .map_err(|error| format!("control channel connection failed: {error}"))?;

    let ready = receive_server_message(&mut socket, Duration::from_secs(10)).await?;
    if ready.message_type != "session_ready" {
        return Err(format!(
            "control channel expected session_ready, received {}",
            ready.message_type
        ));
    }
    let mut sequence = ready.sequence_ack.max(session.sequence_last);
    let interval = Duration::from_secs(u64::from(session.heartbeat_interval_seconds).max(1));
    let response_timeout = Duration::from_secs(
        u64::from(session.heartbeat_interval_seconds)
            .saturating_mul(u64::from(session.missed_heartbeat_limit))
            .max(1),
    );

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let _ = socket.close(None).await;
                return Ok(ConnectionOutcome::Stopped);
            }
            _ = tokio::time::sleep(interval) => {}
        }

        sequence = sequence.saturating_add(1);
        let registration = build_registration_payload(env!("CARGO_PKG_VERSION"));
        let heartbeat = ClientControlMessage {
            session_id: session.session_id.clone(),
            device_id: enrollment.device_id.clone(),
            sequence,
            sent_at: Utc::now().to_rfc3339(),
            message_type: "heartbeat".to_string(),
            payload: serde_json::to_value(HeartbeatPayload {
                hardware_fingerprint: registration.hardware_fingerprint,
                local_status: serde_json::json!({
                    "agent": "connected",
                    "credential_expires_at": enrollment.credential_expires_at,
                }),
            })
            .map_err(|error| format!("failed to serialize heartbeat: {error}"))?,
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&heartbeat)
                    .map_err(|error| format!("failed to serialize heartbeat: {error}"))?
                    .into(),
            ))
            .await
            .map_err(|error| format!("failed to send heartbeat: {error}"))?;

        let response = tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let _ = socket.close(None).await;
                return Ok(ConnectionOutcome::Stopped);
            }
            response = receive_server_message(&mut socket, response_timeout) => response?
        };
        match response.message_type.as_str() {
            "heartbeat_ack" => {
                if response.sequence_ack != sequence {
                    return Err(format!(
                        "heartbeat acknowledgement mismatch: sent {sequence}, received {}",
                        response.sequence_ack
                    ));
                }
                update_remote_session_sequence(sequence)?;
            }
            "session_revoked" => {
                return Ok(ConnectionOutcome::Terminal(
                    "remote session was revoked by the control plane".to_string(),
                ));
            }
            "error" => {
                let message = response.payload["message"]
                    .as_str()
                    .unwrap_or("control plane rejected heartbeat");
                return Err(message.to_string());
            }
            other => return Err(format!("unexpected control message {other}")),
        }

        if credential_refresh_due(&enrollment)? {
            enrollment = tokio::task::spawn_blocking(|| {
                refresh_credential()?;
                load_remote_enrollment()
            })
            .await
            .map_err(|error| format!("credential refresh task failed: {error}"))??;
        }
    }
}

async fn receive_server_message<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    timeout: Duration,
) -> Result<ServerControlMessage, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let incoming = tokio::time::timeout(timeout, socket.next())
            .await
            .map_err(|_| "control channel heartbeat timeout".to_string())?;
        match incoming {
            None => return Err("control channel closed".to_string()),
            Some(Err(error)) => return Err(format!("control channel read failed: {error}")),
            Some(Ok(Message::Close(_))) => return Err("control channel closed".to_string()),
            Some(Ok(Message::Ping(payload))) => socket
                .send(Message::Pong(payload))
                .await
                .map_err(|error| format!("failed to answer WebSocket ping: {error}"))?,
            Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
            Some(Ok(Message::Binary(_))) => {
                return Err("control plane sent an unsupported binary message".to_string());
            }
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text)
                    .map_err(|error| format!("invalid server control message: {error}"));
            }
        }
    }
}

fn ensure_credential_fresh() -> Result<RemoteEnrollmentState, String> {
    let state = load_remote_enrollment()?;
    if credential_refresh_due(&state)? {
        refresh_credential()?;
        load_remote_enrollment()
    } else {
        Ok(state)
    }
}

fn credential_refresh_due(state: &RemoteEnrollmentState) -> Result<bool, String> {
    let expires_at = chrono::DateTime::parse_from_rfc3339(&state.credential_expires_at)
        .map_err(|error| format!("invalid credential expiry: {error}"))?;
    Ok(expires_at <= Utc::now() + ChronoDuration::minutes(2))
}

fn remote_error(value: &serde_json::Value) -> String {
    let code = value["error"]["code"].as_str().unwrap_or("remote_error");
    let message = value["error"]["message"]
        .as_str()
        .unwrap_or("control plane rejected request");
    format!("control plane {code}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_refreshes_before_expiry() {
        let state = RemoteEnrollmentState {
            control_plane_url: "https://api.burd.cloud".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            public_key_id: "key_1".to_string(),
            credential: "secret".to_string(),
            credential_expires_at: (Utc::now() + ChronoDuration::seconds(30)).to_rfc3339(),
            enrolled_at: Utc::now().to_rfc3339(),
        };
        assert!(credential_refresh_due(&state).unwrap());
    }
}
