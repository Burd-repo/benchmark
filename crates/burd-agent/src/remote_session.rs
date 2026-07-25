use crate::remote_enrollment::{
    ControlPlaneRequestError, join_url, post_json_checked, refresh_credential,
    refresh_credential_checked,
};
use burd_bench::{ProviderRegistrationPayload, build_registration_payload};
use burd_hardware::collect_nvidia_telemetry;
use burd_protocol::{
    ClientControlMessage, GpuTelemetrySample, HeartbeatPayload, RemoteEnrollmentState,
    RemoteSessionRecord, RemoteSessionResume, RemoteSessionState, RemoteSessionStateStatus,
    ServerControlMessage, SignedTelemetryBatch, StartRemoteSessionRequest,
    StartRemoteSessionResponse, TELEMETRY_CANONICALIZATION_VERSION, TELEMETRY_SCHEMA_VERSION,
    TelemetryBatchPayload, TelemetryBatchReceipt, clear_remote_session, load_identity,
    load_private_key, load_remote_enrollment, load_remote_session, save_remote_session,
    show_remote_session, sign_message, telemetry_batch_hash, telemetry_batch_signature_message,
    update_remote_session_sequence, update_remote_telemetry_sequence,
};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};

pub fn connect(
    agent_version: &str,
    max_reconnect_delay_seconds: u64,
    telemetry: bool,
    telemetry_batch_samples: usize,
) -> Result<RemoteSessionStateStatus, String> {
    validate_telemetry_batch_samples(telemetry_batch_samples)?;
    let identity = load_identity()?;
    let telemetry_enabled = telemetry || identity.telemetry_enabled;
    let retry_seed = stable_retry_seed(&identity.machine_id);
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| format!("failed to start remote session runtime: {error}"))?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    runtime.block_on(async move {
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = shutdown_tx.send(true);
        });
        run_control_loop(
            agent_version.to_string(),
            max_reconnect_delay_seconds.max(1),
            retry_seed,
            telemetry_enabled,
            telemetry_batch_samples,
            shutdown_rx,
        )
        .await
    })?;
    show_remote_session()
}

#[cfg(feature = "integration-test-support")]
pub async fn connect_until_shutdown(
    agent_version: &str,
    max_reconnect_delay_seconds: u64,
    telemetry: bool,
    telemetry_batch_samples: usize,
    shutdown: watch::Receiver<bool>,
) -> Result<RemoteSessionStateStatus, String> {
    validate_telemetry_batch_samples(telemetry_batch_samples)?;
    let identity = tokio::task::spawn_blocking(load_identity)
        .await
        .map_err(|error| format!("failed to load identity task: {error}"))??;
    let telemetry_enabled = telemetry || identity.telemetry_enabled;
    let retry_seed = stable_retry_seed(&identity.machine_id);
    run_control_loop(
        agent_version.to_string(),
        max_reconnect_delay_seconds.max(1),
        retry_seed,
        telemetry_enabled,
        telemetry_batch_samples,
        shutdown,
    )
    .await?;
    tokio::task::spawn_blocking(show_remote_session)
        .await
        .map_err(|error| format!("failed to load remote session status task: {error}"))?
}

fn validate_telemetry_batch_samples(telemetry_batch_samples: usize) -> Result<(), String> {
    if !(1..=64).contains(&telemetry_batch_samples) {
        return Err("telemetry_batch_samples must be between 1 and 64".to_string());
    }
    Ok(())
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

fn start_or_resume(
    agent_version: &str,
    registration: &ProviderRegistrationPayload,
) -> Result<RemoteSessionStateStatus, ReconnectFailure> {
    let enrollment = load_remote_enrollment().map_err(|error| {
        ReconnectFailure::terminal("local_state", format!("failed to load enrollment: {error}"))
    })?;
    let persisted = load_remote_session().ok();
    let request = StartRemoteSessionRequest {
        provider_id: enrollment.provider_id.clone(),
        device_id: enrollment.device_id.clone(),
        hardware_fingerprint: registration.hardware_fingerprint.clone(),
        agent_version: agent_version.to_string(),
        capabilities: registration.capabilities.clone(),
        latest_report_hash: registration.latest_signed_report_hash.clone(),
        latest_challenge_id: None,
        resume: persisted.as_ref().map(|state| RemoteSessionResume {
            session_id: state.session_id.clone(),
            resume_token: state.resume_token.clone(),
        }),
    };
    let result: Result<StartRemoteSessionResponse, ControlPlaneRequestError> = post_json_checked(
        &join_url(&enrollment.control_plane_url, "/v1/sessions"),
        &request,
        Some(&enrollment.credential),
    );
    let response = match result {
        Ok(response) => response,
        Err(error) if persisted.is_some() && persisted_session_should_restart(&error) => {
            clear_remote_session().map_err(|error| {
                ReconnectFailure::terminal(
                    "local_state",
                    format!("failed to clear expired remote session: {error}"),
                )
            })?;
            return start_or_resume(agent_version, registration);
        }
        Err(error) => return Err(classify_control_plane_error(error)),
    };
    save_remote_session(&enrollment.control_plane_url, &response).map_err(|error| {
        ReconnectFailure::terminal(
            "local_state",
            format!("failed to persist remote session: {error}"),
        )
    })
}

async fn run_control_loop(
    agent_version: String,
    max_reconnect_delay_seconds: u64,
    retry_seed: u64,
    telemetry_enabled: bool,
    telemetry_batch_samples: usize,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut retry_policy = ReconnectPolicy::new(max_reconnect_delay_seconds, retry_seed);
    loop {
        if shutdown_requested(&shutdown) {
            return Ok(());
        }
        match attempt_connection(
            agent_version.clone(),
            telemetry_enabled,
            telemetry_batch_samples,
            &mut shutdown,
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(failure) => {
                match failure.disposition {
                    ReconnectDisposition::Stop => return Err(failure.message),
                    ReconnectDisposition::RestartSession => {
                        clear_remote_session().map_err(|error| {
                            format!("failed to clear remote session before restart: {error}")
                        })?;
                    }
                    ReconnectDisposition::Retry => {}
                }
                let delay = retry_policy.delay_after_failure(failure.connection_was_stable);
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "remote_session_retry_scheduled",
                        "attempt": delay.attempt,
                        "delay_seconds": delay.seconds,
                        "backoff_ceiling_seconds": delay.ceiling_seconds,
                        "failure_kind": failure.kind,
                        "action": failure.disposition.as_str(),
                        "connection_was_stable": failure.connection_was_stable,
                        "error": failure.message,
                    })
                );
                tokio::select! {
                    _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_secs(delay.seconds)) => {}
                }
            }
        }
    }
}

async fn attempt_connection(
    agent_version: String,
    telemetry_enabled: bool,
    telemetry_batch_samples: usize,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), ReconnectFailure> {
    if shutdown_requested(shutdown) {
        return Ok(());
    }
    let prepared = tokio::task::spawn_blocking(move || {
        let enrollment = ensure_credential_fresh()?;
        let registration = build_registration_payload(&agent_version);
        let hardware_fingerprint = registration.hardware_fingerprint.clone();
        start_or_resume(&agent_version, &registration)?;
        let session = load_remote_session().map_err(|error| {
            ReconnectFailure::terminal(
                "local_state",
                format!("failed to load remote session: {error}"),
            )
        })?;
        Ok::<_, ReconnectFailure>((enrollment, session, hardware_fingerprint))
    })
    .await
    .map_err(|error| {
        ReconnectFailure::terminal(
            "internal_task",
            format!("remote session preparation task failed: {error}"),
        )
    })??;

    let (enrollment, session, hardware_fingerprint) = prepared;
    let mut connection_was_stable = false;
    match run_one_connection(
        enrollment,
        session,
        hardware_fingerprint,
        telemetry_enabled,
        telemetry_batch_samples,
        &mut connection_was_stable,
        shutdown,
    )
    .await
    {
        Ok(ConnectionOutcome::Stopped) => Ok(()),
        Ok(ConnectionOutcome::Terminal(reason)) => {
            Err(ReconnectFailure::terminal("session_revoked", reason)
                .with_connection_stability(connection_was_stable))
        }
        Ok(ConnectionOutcome::RestartSession(reason)) => {
            Err(ReconnectFailure::restart_session("session_expired", reason)
                .with_connection_stability(connection_was_stable))
        }
        Err(error) => Err(ReconnectFailure::retry("connection_error", error)
            .with_connection_stability(connection_was_stable)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconnectDisposition {
    Retry,
    RestartSession,
    Stop,
}

impl ReconnectDisposition {
    fn as_str(self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::RestartSession => "restart_session",
            Self::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReconnectFailure {
    kind: &'static str,
    message: String,
    disposition: ReconnectDisposition,
    connection_was_stable: bool,
}

impl ReconnectFailure {
    fn retry(kind: &'static str, message: String) -> Self {
        Self {
            kind,
            message,
            disposition: ReconnectDisposition::Retry,
            connection_was_stable: false,
        }
    }

    fn restart_session(kind: &'static str, message: String) -> Self {
        Self {
            kind,
            message,
            disposition: ReconnectDisposition::RestartSession,
            connection_was_stable: false,
        }
    }

    fn terminal(kind: &'static str, message: String) -> Self {
        Self {
            kind,
            message,
            disposition: ReconnectDisposition::Stop,
            connection_was_stable: false,
        }
    }

    fn with_connection_stability(mut self, stable: bool) -> Self {
        self.connection_was_stable = stable;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetryDelay {
    attempt: u32,
    seconds: u64,
    ceiling_seconds: u64,
}

#[derive(Debug, Clone)]
struct ReconnectPolicy {
    max_delay_seconds: u64,
    consecutive_failures: u32,
    jitter_seed: u64,
}

impl ReconnectPolicy {
    fn new(max_delay_seconds: u64, jitter_seed: u64) -> Self {
        Self {
            max_delay_seconds: max_delay_seconds.max(1),
            consecutive_failures: 0,
            jitter_seed,
        }
    }

    fn delay_after_failure(&mut self, connection_was_stable: bool) -> RetryDelay {
        if connection_was_stable {
            self.reset();
        }
        self.next_delay()
    }

    fn next_delay(&mut self) -> RetryDelay {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let exponent = self.consecutive_failures.saturating_sub(1).min(63);
        let ceiling_seconds = 1_u64
            .checked_shl(exponent)
            .unwrap_or(u64::MAX)
            .min(self.max_delay_seconds);
        let floor_seconds = (ceiling_seconds / 2).max(1);
        let jitter_span = ceiling_seconds.saturating_sub(floor_seconds);
        let jitter = if jitter_span == 0 {
            0
        } else {
            mix_retry_seed(self.jitter_seed ^ u64::from(self.consecutive_failures))
                % (jitter_span + 1)
        };
        RetryDelay {
            attempt: self.consecutive_failures,
            seconds: floor_seconds + jitter,
            ceiling_seconds,
        }
    }

    fn reset(&mut self) {
        self.consecutive_failures = 0;
    }
}

fn stable_retry_seed(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn mix_retry_seed(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58476d1ce4e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn persisted_session_should_restart(error: &ControlPlaneRequestError) -> bool {
    error.is_code("expired")
        || error.is_code("not_found")
        || matches!(
            error,
            ControlPlaneRequestError::Rejected {
                status: 404 | 410,
                ..
            }
        )
}

fn classify_control_plane_error(error: ControlPlaneRequestError) -> ReconnectFailure {
    match error {
        ControlPlaneRequestError::LocalState(message) => {
            ReconnectFailure::terminal("local_state", message)
        }
        ControlPlaneRequestError::Transport(message) => {
            ReconnectFailure::retry("control_plane_transport", message)
        }
        ControlPlaneRequestError::Contract(message) => {
            ReconnectFailure::terminal("control_plane_contract", message)
        }
        ControlPlaneRequestError::Rejected {
            status,
            code,
            message,
        } => {
            let detail = format!("control plane {code}: {message}");
            if code == "revoked" {
                ReconnectFailure::terminal("revoked", detail)
            } else if code == "unauthorized" || status == 401 || status == 403 {
                ReconnectFailure::terminal("unauthorized", detail)
            } else if code == "expired" {
                ReconnectFailure::terminal("expired", detail)
            } else if code == "not_found" {
                ReconnectFailure::terminal("not_found", detail)
            } else if status == 408 || status == 429 || status >= 500 || code == "conflict" {
                ReconnectFailure::retry("control_plane_rejected", detail)
            } else {
                ReconnectFailure::terminal("control_plane_rejected", detail)
            }
        }
    }
}

enum ConnectionOutcome {
    Stopped,
    Terminal(String),
    RestartSession(String),
}

fn websocket_http_outcome(status: u16) -> Option<ConnectionOutcome> {
    match status {
        400 => Some(ConnectionOutcome::Terminal(
            "control plane rejected the WebSocket request contract".to_string(),
        )),
        401 => Some(ConnectionOutcome::Terminal(
            "control channel credential is invalid or expired".to_string(),
        )),
        403 => Some(ConnectionOutcome::Terminal(
            "control channel device or session has been revoked".to_string(),
        )),
        404 | 410 => Some(ConnectionOutcome::RestartSession(
            "remote session is missing or expired".to_string(),
        )),
        _ => None,
    }
}

async fn run_one_connection(
    mut enrollment: RemoteEnrollmentState,
    session: RemoteSessionState,
    hardware_fingerprint: String,
    telemetry_enabled: bool,
    telemetry_batch_samples: usize,
    connection_was_stable: &mut bool,
    shutdown: &mut watch::Receiver<bool>,
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
    let (mut socket, _) = match connect_async(request).await {
        Ok(connected) => connected,
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            if let Some(outcome) = websocket_http_outcome(response.status().as_u16()) {
                return Ok(outcome);
            }
            return Err(format!(
                "control channel handshake failed with HTTP {}",
                response.status().as_u16()
            ));
        }
        Err(error) => return Err(format!("control channel connection failed: {error}")),
    };

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
    let mut telemetry_samples = Vec::<GpuTelemetrySample>::new();
    let mut telemetry_sequence = session.telemetry_sequence_last;
    let mut telemetry_unavailable_logged = false;
    let mut telemetry_active = telemetry_enabled;

    loop {
        tokio::select! {
            _ = wait_for_shutdown(shutdown) => {
                let _ = socket.close(None).await;
                return Ok(ConnectionOutcome::Stopped);
            }
            _ = tokio::time::sleep(interval) => {}
        }

        sequence = sequence.saturating_add(1);
        let heartbeat = ClientControlMessage {
            session_id: session.session_id.clone(),
            device_id: enrollment.device_id.clone(),
            sequence,
            sent_at: Utc::now().to_rfc3339(),
            message_type: "heartbeat".to_string(),
            payload: serde_json::to_value(HeartbeatPayload {
                hardware_fingerprint: hardware_fingerprint.clone(),
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
            _ = wait_for_shutdown(shutdown) => {
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
                *connection_was_stable = true;
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

        if telemetry_active {
            let next_sample_sequence = telemetry_sequence
                .checked_add(telemetry_samples.len() as u64 + 1)
                .ok_or_else(|| "GPU telemetry sequence overflow".to_string())?;
            match tokio::task::spawn_blocking(move || {
                collect_nvidia_telemetry(next_sample_sequence)
            })
            .await
            .map_err(|error| format!("GPU telemetry collection task failed: {error}"))?
            {
                Ok(collection) => {
                    telemetry_unavailable_logged = false;
                    for warning in collection.warnings {
                        eprintln!(
                            "{}",
                            serde_json::json!({
                                "event": "gpu_telemetry_partial",
                                "warning": warning,
                            })
                        );
                    }
                    telemetry_samples.extend(collection.samples);
                    if telemetry_samples.len() > 64 {
                        return Err("GPU telemetry batch exceeded 64 samples".to_string());
                    }
                    if telemetry_samples.len() >= telemetry_batch_samples {
                        sequence = sequence.saturating_add(1);
                        let signed = build_signed_telemetry_batch(
                            &enrollment,
                            &session,
                            &hardware_fingerprint,
                            sequence,
                            collection.collector,
                            &telemetry_samples,
                        )?;
                        let batch_hash = signed.batch_hash.clone();
                        let telemetry_message = ClientControlMessage {
                            session_id: session.session_id.clone(),
                            device_id: enrollment.device_id.clone(),
                            sequence,
                            sent_at: Utc::now().to_rfc3339(),
                            message_type: "telemetry_batch".to_string(),
                            payload: serde_json::to_value(signed).map_err(|error| {
                                format!("failed to serialize telemetry batch: {error}")
                            })?,
                        };
                        socket
                            .send(Message::Text(
                                serde_json::to_string(&telemetry_message)
                                    .map_err(|error| {
                                        format!("failed to serialize telemetry message: {error}")
                                    })?
                                    .into(),
                            ))
                            .await
                            .map_err(|error| format!("failed to send telemetry batch: {error}"))?;
                        let response = tokio::select! {
                            _ = wait_for_shutdown(shutdown) => {
                                let _ = socket.close(None).await;
                                return Ok(ConnectionOutcome::Stopped);
                            }
                            response = receive_server_message(&mut socket, response_timeout) => response?
                        };
                        match response.message_type.as_str() {
                            "telemetry_ack" => {
                                let receipt: TelemetryBatchReceipt =
                                    serde_json::from_value(response.payload).map_err(|error| {
                                        format!("invalid telemetry acknowledgement: {error}")
                                    })?;
                                let expected_sample_end = telemetry_samples
                                    .last()
                                    .map(|sample| sample.sample_sequence)
                                    .ok_or_else(|| "telemetry batch is empty".to_string())?;
                                if receipt.control_sequence_ack != sequence
                                    || receipt.sample_sequence_end != expected_sample_end
                                    || receipt.batch_hash != batch_hash
                                {
                                    return Err(
                                        "telemetry acknowledgement does not match sent batch"
                                            .to_string(),
                                    );
                                }
                                update_remote_session_sequence(sequence)?;
                                update_remote_telemetry_sequence(expected_sample_end)?;
                                telemetry_sequence = expected_sample_end;
                                telemetry_samples.clear();
                            }
                            "telemetry_rejected" => {
                                let message = response.payload["message"]
                                    .as_str()
                                    .unwrap_or("control plane rejected telemetry batch");
                                eprintln!(
                                    "{}",
                                    serde_json::json!({
                                        "event": "gpu_telemetry_rejected",
                                        "error": message,
                                    })
                                );
                                sequence = response.sequence_ack;
                                update_remote_session_sequence(sequence)?;
                                telemetry_samples.clear();
                                telemetry_active = false;
                            }
                            "session_revoked" => {
                                return Ok(ConnectionOutcome::Terminal(
                                    "remote session was revoked by the control plane".to_string(),
                                ));
                            }
                            "error" => {
                                let message = response.payload["message"]
                                    .as_str()
                                    .unwrap_or("control plane rejected telemetry batch");
                                return Err(message.to_string());
                            }
                            other => {
                                return Err(format!(
                                    "unexpected telemetry control message {other}"
                                ));
                            }
                        }
                    }
                }
                Err(error) => {
                    if !telemetry_unavailable_logged {
                        eprintln!(
                            "{}",
                            serde_json::json!({
                                "event": "gpu_telemetry_unavailable",
                                "error": error,
                            })
                        );
                        telemetry_unavailable_logged = true;
                    }
                }
            }
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

fn shutdown_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !shutdown_requested(shutdown) {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

fn build_signed_telemetry_batch(
    enrollment: &RemoteEnrollmentState,
    session: &RemoteSessionState,
    hardware_fingerprint: &str,
    control_sequence: u64,
    collector: String,
    samples: &[GpuTelemetrySample],
) -> Result<SignedTelemetryBatch, String> {
    let first = samples
        .first()
        .ok_or_else(|| "telemetry batch requires at least one sample".to_string())?;
    let last = samples
        .last()
        .ok_or_else(|| "telemetry batch requires at least one sample".to_string())?;
    let identity = load_identity()?;
    let private_key = load_private_key(&identity)?;
    let payload = TelemetryBatchPayload {
        schema_version: TELEMETRY_SCHEMA_VERSION.to_string(),
        provider_id: enrollment.provider_id.clone(),
        device_id: enrollment.device_id.clone(),
        session_id: session.session_id.clone(),
        control_sequence,
        sample_sequence_start: first.sample_sequence,
        sample_sequence_end: last.sample_sequence,
        hardware_fingerprint: hardware_fingerprint.to_string(),
        collector,
        collected_at_start: first.observed_at.clone(),
        collected_at_end: last.observed_at.clone(),
        samples: samples.to_vec(),
    };
    let batch_hash = telemetry_batch_hash(&payload)?;
    let message =
        telemetry_batch_signature_message(&payload, &batch_hash, &enrollment.public_key_id)?;
    let signature = sign_message(&private_key.secret_key_base64, message.as_bytes())?;
    Ok(SignedTelemetryBatch {
        payload,
        batch_hash,
        public_key_id: enrollment.public_key_id.clone(),
        signature,
        canonicalization_version: TELEMETRY_CANONICALIZATION_VERSION.to_string(),
    })
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

fn ensure_credential_fresh() -> Result<RemoteEnrollmentState, ReconnectFailure> {
    let state = load_remote_enrollment().map_err(|error| {
        ReconnectFailure::terminal(
            "local_state",
            format!("failed to load remote enrollment: {error}"),
        )
    })?;
    let refresh_due = credential_refresh_due(&state).map_err(|error| {
        ReconnectFailure::terminal(
            "local_state",
            format!("failed to evaluate credential expiry: {error}"),
        )
    })?;
    if refresh_due {
        refresh_credential_checked().map_err(classify_control_plane_error)?;
        load_remote_enrollment().map_err(|error| {
            ReconnectFailure::terminal(
                "local_state",
                format!("failed to load refreshed credential: {error}"),
            )
        })
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
    fn telemetry_batch_sample_limit_is_validated_before_connecting() {
        assert!(validate_telemetry_batch_samples(1).is_ok());
        assert!(validate_telemetry_batch_samples(64).is_ok());
        assert!(validate_telemetry_batch_samples(0).is_err());
        assert!(validate_telemetry_batch_samples(65).is_err());
    }

    #[tokio::test]
    async fn shutdown_receiver_wakes_control_loop_waiters() {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(
            Duration::from_millis(100),
            wait_for_shutdown(&mut shutdown_rx),
        )
        .await
        .unwrap();
        assert!(shutdown_requested(&shutdown_rx));
    }

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

    #[test]
    fn reconnect_backoff_is_deterministic_bounded_and_resettable() {
        let mut first = ReconnectPolicy::new(16, 42);
        let mut second = ReconnectPolicy::new(16, 42);
        let first_delays = (0..7).map(|_| first.next_delay()).collect::<Vec<_>>();
        let second_delays = (0..7).map(|_| second.next_delay()).collect::<Vec<_>>();

        assert_eq!(first_delays, second_delays);
        assert_eq!(
            first_delays
                .iter()
                .map(|delay| delay.ceiling_seconds)
                .collect::<Vec<_>>(),
            vec![1, 2, 4, 8, 16, 16, 16]
        );
        for delay in &first_delays {
            assert!(delay.seconds >= (delay.ceiling_seconds / 2).max(1));
            assert!(delay.seconds <= delay.ceiling_seconds);
        }

        assert_eq!(
            first.delay_after_failure(true),
            RetryDelay {
                attempt: 1,
                seconds: 1,
                ceiling_seconds: 1,
            }
        );
        assert_eq!(first.delay_after_failure(false).attempt, 2);
    }

    #[test]
    fn control_plane_failures_distinguish_retry_from_terminal_auth() {
        let unavailable = classify_control_plane_error(ControlPlaneRequestError::Rejected {
            status: 503,
            code: "database_unavailable".to_string(),
            message: "dependency unavailable".to_string(),
        });
        assert_eq!(unavailable.disposition, ReconnectDisposition::Retry);
        assert_eq!(unavailable.kind, "control_plane_rejected");

        let revoked = classify_control_plane_error(ControlPlaneRequestError::Rejected {
            status: 403,
            code: "revoked".to_string(),
            message: "device revoked".to_string(),
        });
        assert_eq!(revoked.disposition, ReconnectDisposition::Stop);
        assert_eq!(revoked.kind, "revoked");

        let unauthorized = classify_control_plane_error(ControlPlaneRequestError::Rejected {
            status: 401,
            code: "unauthorized".to_string(),
            message: "credential invalid".to_string(),
        });
        assert_eq!(unauthorized.disposition, ReconnectDisposition::Stop);
        assert_eq!(unauthorized.kind, "unauthorized");
    }

    #[test]
    fn websocket_statuses_restart_expired_sessions_and_stop_revoked_devices() {
        assert!(matches!(
            websocket_http_outcome(410),
            Some(ConnectionOutcome::RestartSession(_))
        ));
        assert!(matches!(
            websocket_http_outcome(403),
            Some(ConnectionOutcome::Terminal(_))
        ));
        assert!(websocket_http_outcome(503).is_none());

        let missing_without_envelope = ControlPlaneRequestError::Rejected {
            status: 404,
            code: "remote_error".to_string(),
            message: "control plane rejected request".to_string(),
        };
        assert!(persisted_session_should_restart(&missing_without_envelope));
    }
}
