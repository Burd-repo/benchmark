mod cuda;
mod ollama;
mod state;

use self::state::{ProofAttemptOutcome, ProofAttemptStore};
use crate::remote_enrollment::{ControlPlaneRequestError, join_url};
use burd_bench::build_registration_payload;
use burd_protocol::{
    GpuTelemetrySample, NextProofChallengeResponse, PROOF_CAPABILITY_REQUIRED_PROOFS,
    PROOF_CHALLENGE_CANONICALIZATION_VERSION, PROOF_CHALLENGE_RESPONSE_SCHEMA_VERSION,
    PROOF_CHALLENGE_SCHEMA_VERSION, ProofCapabilityChallenge, ProofCapabilityMetrics,
    ProofCapabilityResponsePayload, SignedProofCapabilityResponse, SubmitProofChallengeResponse,
    load_identity, load_private_key, load_remote_enrollment, load_remote_session,
    proof_capability_response_hash, proof_capability_response_signature_message, sign_message,
};
use chrono::{DateTime, Utc};
use std::collections::BTreeSet;
use std::sync::mpsc as std_mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

pub(crate) use cuda::execute_remote_proof;

const PROOF_POLL_INTERVAL: Duration = Duration::from_secs(5);
const TELEMETRY_CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
const EXECUTION_GATE_TIMEOUT: Duration = Duration::from_secs(30);
const PROOF_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
#[derive(Debug, Clone)]
pub(crate) struct ProofTelemetryWindow {
    pub(crate) batch_hash: String,
    pub(crate) samples: Vec<GpuTelemetrySample>,
}

#[derive(Debug)]
pub(crate) struct ProofTelemetryRequest {
    pub(crate) required_gpu_uuid: String,
    pub(crate) response: oneshot::Sender<Result<ProofTelemetryWindow, String>>,
}

#[derive(Debug)]
pub struct ProofExecutionRequest {
    pub challenge: ProofCapabilityChallenge,
    ready: Option<oneshot::Sender<ProofExecutionReady>>,
    continue_execution: std_mpsc::Receiver<bool>,
    cancellation: ProofCancellation,
}

impl ProofExecutionRequest {
    pub fn cancellation_requested(&self) -> bool {
        self.cancellation.requested()
    }

    pub fn ensure_not_cancelled(&self) -> Result<(), String> {
        self.cancellation.ensure_not_cancelled()
    }

    pub fn hold_residency_for_telemetry(&mut self, gpu_uuid: String) -> Result<(), String> {
        self.ensure_not_cancelled()?;
        let ready = self
            .ready
            .take()
            .ok_or_else(|| "proof telemetry readiness was already signaled".to_string())?;
        ready
            .send(ProofExecutionReady { gpu_uuid })
            .map_err(|_| "proof worker stopped before telemetry capture".to_string())?;
        match self.continue_execution.recv_timeout(EXECUTION_GATE_TIMEOUT) {
            Ok(true) => self.ensure_not_cancelled(),
            Ok(false) => {
                Err("proof execution cancelled because telemetry was not accepted".to_string())
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                Err("proof telemetry capture timed out while VRAM was resident".to_string())
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                Err("proof worker disconnected during telemetry capture".to_string())
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ProofCancellation {
    requested: Arc<AtomicBool>,
}

impl ProofCancellation {
    fn cancel(&self) {
        self.requested.store(true, Ordering::Release);
    }

    fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    fn ensure_not_cancelled(&self) -> Result<(), String> {
        if self.requested() {
            Err("proof execution cancelled by Agent shutdown".to_string())
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProofExecution {
    pub gpu_uuid: String,
    pub driver_version: String,
    pub cuda_driver_version: Option<String>,
    pub cuda_runtime_version: Option<String>,
    pub metrics: ProofCapabilityMetrics,
}

#[derive(Debug)]
struct ProofExecutionReady {
    gpu_uuid: String,
}

pub type ProofExecutor = fn(ProofExecutionRequest) -> Result<ProofExecution, String>;

pub(crate) async fn run_worker(
    agent_version: String,
    telemetry: mpsc::Sender<ProofTelemetryRequest>,
    executor: ProofExecutor,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut attempts = tokio::task::spawn_blocking(ProofAttemptStore::load_default)
        .await
        .map_err(|error| format!("failed to load proof attempt state task: {error}"))??;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }

        let current_session = tokio::task::spawn_blocking(load_remote_session)
            .await
            .map_err(|error| format!("failed to load proof session task: {error}"))?;
        let current_session = match current_session {
            Ok(session) => session,
            Err(_) => {
                wait_for_poll_or_shutdown(&mut shutdown).await;
                continue;
            }
        };
        if attempts
            .active_suppression(&current_session.session_id, Utc::now())
            .is_some()
        {
            wait_for_poll_or_shutdown(&mut shutdown).await;
            continue;
        }
        let fetched = tokio::task::spawn_blocking(fetch_next_challenge)
            .await
            .map_err(|error| format!("proof challenge fetch task failed: {error}"))?;
        let challenge = match fetched {
            Ok(challenge) => challenge,
            Err(error) if error.is_code("not_found") => {
                wait_for_poll_or_shutdown(&mut shutdown).await;
                continue;
            }
            Err(error) => {
                log_proof_event("remote_proof_fetch_failed", None, &error.to_string());
                wait_for_poll_or_shutdown(&mut shutdown).await;
                continue;
            }
        };

        let fingerprint_agent_version = agent_version.clone();
        let current_fingerprint = tokio::task::spawn_blocking(move || {
            build_registration_payload(&fingerprint_agent_version).hardware_fingerprint
        })
        .await
        .map_err(|error| format!("failed to collect proof hardware fingerprint: {error}"))?;
        let expires_at = match validate_challenge_context(&challenge, &current_fingerprint) {
            Ok(expires_at) => expires_at,
            Err(error) => {
                log_proof_event(
                    "remote_proof_rejected_locally",
                    Some(&challenge.challenge_id),
                    &error,
                );
                let recorded_at = Utc::now();
                let suppress_until = suppression_deadline(&challenge.expires_at, recorded_at);
                attempts = persist_attempt(
                    attempts,
                    challenge.challenge_id,
                    challenge.session_id,
                    ProofAttemptOutcome::RejectedLocally,
                    recorded_at,
                    suppress_until,
                )
                .await?;
                continue;
            }
        };

        let challenge_id = challenge.challenge_id.clone();
        let session_id = challenge.session_id.clone();
        match execute_and_submit_challenge(challenge, &telemetry, executor, &mut shutdown).await {
            Ok(Some(response)) => {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "remote_proof_submitted",
                        "challenge_id": response.challenge_id,
                        "status": response.status,
                        "response_hash": response.response_hash,
                    })
                );
                let recorded_at = Utc::now();
                attempts = persist_attempt(
                    attempts,
                    challenge_id,
                    session_id,
                    ProofAttemptOutcome::Submitted,
                    recorded_at,
                    recorded_at,
                )
                .await?;
            }
            Ok(None) => return Ok(()),
            Err(error) => {
                log_proof_event("remote_proof_execution_failed", Some(&challenge_id), &error);
                let recorded_at = Utc::now();
                let suppress_until = if expires_at > recorded_at {
                    expires_at
                } else {
                    recorded_at + chrono::Duration::seconds(PROOF_POLL_INTERVAL.as_secs() as i64)
                };
                attempts = persist_attempt(
                    attempts,
                    challenge_id,
                    session_id,
                    ProofAttemptOutcome::AttemptFailed,
                    recorded_at,
                    suppress_until,
                )
                .await?;
            }
        }
    }
}

async fn persist_attempt(
    mut store: ProofAttemptStore,
    challenge_id: String,
    session_id: String,
    outcome: ProofAttemptOutcome,
    recorded_at: DateTime<Utc>,
    suppress_until: DateTime<Utc>,
) -> Result<ProofAttemptStore, String> {
    tokio::task::spawn_blocking(move || {
        store.record(
            challenge_id,
            session_id,
            outcome,
            recorded_at,
            suppress_until,
        )?;
        Ok(store)
    })
    .await
    .map_err(|error| format!("failed to persist proof attempt state task: {error}"))?
}

fn suppression_deadline(value: &str, recorded_at: DateTime<Utc>) -> DateTime<Utc> {
    parse_utc(value)
        .ok()
        .filter(|expires_at| *expires_at > recorded_at)
        .unwrap_or(recorded_at + chrono::Duration::seconds(PROOF_POLL_INTERVAL.as_secs() as i64))
}

async fn execute_and_submit_challenge(
    challenge: ProofCapabilityChallenge,
    telemetry: &mpsc::Sender<ProofTelemetryRequest>,
    executor: ProofExecutor,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Option<SubmitProofChallengeResponse>, String> {
    let started_at = Utc::now();
    let (ready_tx, ready_rx) = oneshot::channel();
    let (continue_tx, continue_rx) = std_mpsc::channel();
    let cancellation = ProofCancellation::default();
    let execution_request = ProofExecutionRequest {
        challenge: challenge.clone(),
        ready: Some(ready_tx),
        continue_execution: continue_rx,
        cancellation: cancellation.clone(),
    };
    let mut execution_task = tokio::task::spawn_blocking(move || executor(execution_request));

    let ready_result = tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => {
            stop_proof_execution(
                &cancellation,
                &continue_tx,
                &mut execution_task,
                &challenge.challenge_id,
            ).await;
            return Ok(None);
        }
        result = tokio::time::timeout(TELEMETRY_CAPTURE_TIMEOUT, ready_rx) => result,
    };
    let ready = match ready_result {
        Ok(Ok(ready)) => ready,
        Ok(Err(_)) => {
            let Some(_) = await_proof_execution_or_shutdown(
                &cancellation,
                &continue_tx,
                &mut execution_task,
                &challenge.challenge_id,
                shutdown,
            )
            .await?
            else {
                return Ok(None);
            };
            return Err("proof executor completed without telemetry readiness".to_string());
        }
        Err(_) => {
            cancellation.cancel();
            let _ = continue_tx.send(false);
            return Err(
                "proof executor did not establish VRAM residency before timeout".to_string(),
            );
        }
    };
    if let Some(required) = challenge.required_gpu_uuid.as_deref()
        && !required.eq_ignore_ascii_case(&ready.gpu_uuid)
    {
        cancellation.cancel();
        let _ = continue_tx.send(false);
        return Err(format!(
            "CUDA executor selected GPU {} but challenge requires {required}",
            ready.gpu_uuid
        ));
    }

    let (response_tx, response_rx) = oneshot::channel();
    let telemetry_send = telemetry.send(ProofTelemetryRequest {
        required_gpu_uuid: ready.gpu_uuid.clone(),
        response: response_tx,
    });
    let telemetry_send = tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => {
            stop_proof_execution(
                &cancellation,
                &continue_tx,
                &mut execution_task,
                &challenge.challenge_id,
            ).await;
            return Ok(None);
        }
        result = telemetry_send => result,
    };
    if telemetry_send.is_err() {
        cancellation.cancel();
        let _ = continue_tx.send(false);
        return Err("remote session telemetry channel is unavailable".to_string());
    }
    let response_result = tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => {
            stop_proof_execution(
                &cancellation,
                &continue_tx,
                &mut execution_task,
                &challenge.challenge_id,
            ).await;
            return Ok(None);
        }
        result = tokio::time::timeout(TELEMETRY_CAPTURE_TIMEOUT, response_rx) => result,
    };
    let window = match response_result {
        Ok(Ok(Ok(window))) => window,
        Ok(Ok(Err(error))) => {
            cancellation.cancel();
            let _ = continue_tx.send(false);
            return Err(error);
        }
        Ok(Err(_)) => {
            cancellation.cancel();
            let _ = continue_tx.send(false);
            return Err("remote session dropped the proof telemetry response".to_string());
        }
        Err(_) => {
            cancellation.cancel();
            let _ = continue_tx.send(false);
            return Err("proof telemetry capture timed out".to_string());
        }
    };
    if let Err(error) = validate_telemetry_window(&window, &ready.gpu_uuid, started_at) {
        cancellation.cancel();
        let _ = continue_tx.send(false);
        return Err(error);
    }
    continue_tx
        .send(true)
        .map_err(|_| "proof executor stopped before telemetry was accepted".to_string())?;

    let Some(mut execution) = await_proof_execution_or_shutdown(
        &cancellation,
        &continue_tx,
        &mut execution_task,
        &challenge.challenge_id,
        shutdown,
    )
    .await?
    else {
        return Ok(None);
    };
    if !execution.gpu_uuid.eq_ignore_ascii_case(&ready.gpu_uuid) {
        return Err("proof executor GPU changed after telemetry capture".to_string());
    }
    execution.metrics.contention_detected |= window
        .samples
        .iter()
        .find(|sample| sample.gpu_uuid.eq_ignore_ascii_case(&ready.gpu_uuid))
        .is_some_and(cuda::sample_has_contention);

    let completed_at = Utc::now();
    if completed_at >= parse_utc(&challenge.expires_at)? {
        return Err("proof execution completed after challenge expiration".to_string());
    }
    let signed = build_signed_response(
        &challenge,
        execution,
        window.batch_hash,
        started_at,
        completed_at,
    )?;
    tokio::task::spawn_blocking(move || submit_response(&challenge, &signed))
        .await
        .map_err(|error| format!("proof response submission task failed: {error}"))?
        .map_err(|error| error.to_string())
        .map(Some)
}

async fn await_proof_execution_or_shutdown(
    cancellation: &ProofCancellation,
    continue_tx: &std_mpsc::Sender<bool>,
    execution_task: &mut tokio::task::JoinHandle<Result<ProofExecution, String>>,
    challenge_id: &str,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Option<ProofExecution>, String> {
    tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => {
            stop_proof_execution(cancellation, continue_tx, execution_task, challenge_id).await;
            Ok(None)
        }
        result = &mut *execution_task => {
            result
                .map_err(|error| format!("proof executor task failed: {error}"))?
                .map(Some)
        }
    }
}

async fn stop_proof_execution(
    cancellation: &ProofCancellation,
    continue_tx: &std_mpsc::Sender<bool>,
    execution_task: &mut tokio::task::JoinHandle<Result<ProofExecution, String>>,
    challenge_id: &str,
) {
    cancellation.cancel();
    let _ = continue_tx.send(false);
    if tokio::time::timeout(PROOF_SHUTDOWN_GRACE, &mut *execution_task)
        .await
        .is_err()
    {
        execution_task.abort();
        log_proof_event(
            "remote_proof_shutdown_grace_exceeded",
            Some(challenge_id),
            "proof executor did not stop within the cooperative shutdown grace period",
        );
    }
}
fn validate_challenge_context(
    challenge: &ProofCapabilityChallenge,
    expected_fingerprint: &str,
) -> Result<DateTime<Utc>, String> {
    if challenge.schema_version != PROOF_CHALLENGE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported proof challenge schema {}",
            challenge.schema_version
        ));
    }
    let enrollment = load_remote_enrollment()?;
    let session = load_remote_session()?;
    if challenge.provider_id != enrollment.provider_id
        || challenge.device_id != enrollment.device_id
        || challenge.session_id != session.session_id
    {
        return Err(
            "proof challenge is not bound to the active remote identity and session".to_string(),
        );
    }
    if challenge.required_fingerprint != expected_fingerprint {
        return Err(
            "proof challenge hardware fingerprint does not match current hardware".to_string(),
        );
    }
    if challenge.required_backend != "cuda" {
        return Err(format!(
            "unsupported proof backend {}",
            challenge.required_backend
        ));
    }
    if challenge.required_proofs.is_empty() {
        return Err("proof challenge does not specify any required proofs".to_string());
    }
    let supported = PROOF_CAPABILITY_REQUIRED_PROOFS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = challenge
        .required_proofs
        .iter()
        .find(|proof| !supported.contains(proof.as_str()))
    {
        return Err(format!("unsupported proof requirement {unknown}"));
    }
    if challenge.model_artifact_hash.trim().is_empty()
        || challenge.model_artifact_hash.len() > 160
        || !challenge.model_artifact_hash.is_ascii()
    {
        return Err("proof challenge model artifact hash is invalid".to_string());
    }
    if !challenge.min_tokens_per_second.is_finite() || challenge.min_tokens_per_second < 0.0 {
        return Err("proof challenge token threshold is invalid".to_string());
    }
    let issued_at = parse_utc(&challenge.issued_at)?;
    let expires_at = parse_utc(&challenge.expires_at)?;
    if expires_at <= issued_at || expires_at <= Utc::now() {
        return Err("proof challenge is expired or has an invalid time window".to_string());
    }
    if expires_at - Utc::now() < chrono::Duration::seconds(10) {
        return Err("proof challenge has insufficient execution time remaining".to_string());
    }
    Ok(expires_at)
}

fn validate_telemetry_window(
    window: &ProofTelemetryWindow,
    gpu_uuid: &str,
    started_at: DateTime<Utc>,
) -> Result<(), String> {
    if window.batch_hash.trim().is_empty() {
        return Err("accepted proof telemetry window has an empty batch hash".to_string());
    }
    let sample = window
        .samples
        .iter()
        .rev()
        .find(|sample| sample.gpu_uuid.eq_ignore_ascii_case(gpu_uuid))
        .ok_or_else(|| {
            "accepted proof telemetry window does not include the proof GPU".to_string()
        })?;
    let observed_at = parse_utc(&sample.observed_at)?;
    if observed_at < started_at - chrono::Duration::seconds(2)
        || observed_at > Utc::now() + chrono::Duration::seconds(5)
    {
        return Err("accepted proof telemetry sample is outside the execution window".to_string());
    }
    Ok(())
}
fn build_signed_response(
    challenge: &ProofCapabilityChallenge,
    execution: ProofExecution,
    telemetry_window_hash: String,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
) -> Result<SignedProofCapabilityResponse, String> {
    let enrollment = load_remote_enrollment()?;
    let identity = load_identity()?;
    let private_key = load_private_key(&identity)?;
    let payload = ProofCapabilityResponsePayload {
        schema_version: PROOF_CHALLENGE_RESPONSE_SCHEMA_VERSION.to_string(),
        challenge_id: challenge.challenge_id.clone(),
        nonce: challenge.nonce.clone(),
        provider_id: challenge.provider_id.clone(),
        device_id: challenge.device_id.clone(),
        session_id: challenge.session_id.clone(),
        profile_version: challenge.profile_version.clone(),
        hardware_fingerprint: challenge.required_fingerprint.clone(),
        gpu_uuid: execution.gpu_uuid,
        backend: challenge.required_backend.clone(),
        model_artifact_hash: challenge.model_artifact_hash.clone(),
        prompt_seed: challenge.prompt_seed.clone(),
        driver_version: execution.driver_version,
        cuda_driver_version: execution.cuda_driver_version,
        cuda_runtime_version: execution.cuda_runtime_version,
        metrics: execution.metrics,
        telemetry_window_hash: Some(telemetry_window_hash),
        started_at: started_at.to_rfc3339(),
        completed_at: completed_at.to_rfc3339(),
    };
    let response_hash = proof_capability_response_hash(&payload)?;
    let message = proof_capability_response_signature_message(
        &payload,
        &response_hash,
        &enrollment.public_key_id,
    )?;
    let signature = sign_message(&private_key.secret_key_base64, message.as_bytes())?;
    Ok(SignedProofCapabilityResponse {
        payload,
        response_hash,
        public_key_id: enrollment.public_key_id,
        signature,
        canonicalization_version: PROOF_CHALLENGE_CANONICALIZATION_VERSION.to_string(),
    })
}

fn fetch_next_challenge() -> Result<ProofCapabilityChallenge, ControlPlaneRequestError> {
    let enrollment = load_remote_enrollment().map_err(ControlPlaneRequestError::LocalState)?;
    let session = load_remote_session().map_err(ControlPlaneRequestError::LocalState)?;
    let url = join_url(
        &session.control_plane_url,
        &format!("/v1/sessions/{}/challenges/next", session.session_id),
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
    let mut response = request.call().map_err(|error| {
        ControlPlaneRequestError::Transport(format!("control plane request failed: {error}"))
    })?;
    let status = response.status();
    let value = response.body_mut().read_json::<serde_json::Value>();
    if !status.is_success() {
        return Err(response_error(status.as_u16(), value.ok()));
    }
    let value = value.map_err(|error| {
        ControlPlaneRequestError::Contract(format!(
            "control plane returned invalid proof challenge JSON: {error}"
        ))
    })?;
    let response: NextProofChallengeResponse = serde_json::from_value(value).map_err(|error| {
        ControlPlaneRequestError::Contract(format!(
            "invalid next proof challenge response contract: {error}"
        ))
    })?;
    Ok(response.challenge)
}

fn submit_response(
    challenge: &ProofCapabilityChallenge,
    signed: &SignedProofCapabilityResponse,
) -> Result<SubmitProofChallengeResponse, ControlPlaneRequestError> {
    let enrollment = load_remote_enrollment().map_err(ControlPlaneRequestError::LocalState)?;
    let session = load_remote_session().map_err(ControlPlaneRequestError::LocalState)?;
    if session.session_id != challenge.session_id {
        return Err(ControlPlaneRequestError::LocalState(
            "remote session changed before proof response submission".to_string(),
        ));
    }
    let url = join_url(
        &session.control_plane_url,
        &format!(
            "/v1/sessions/{}/challenges/{}/response",
            session.session_id, challenge.challenge_id
        ),
    );
    let request = ureq::post(&url)
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
    let mut response = request.send_json(signed).map_err(|error| {
        ControlPlaneRequestError::Transport(format!("control plane request failed: {error}"))
    })?;
    let status = response.status();
    let value = response.body_mut().read_json::<serde_json::Value>();
    if !status.is_success() {
        return Err(response_error(status.as_u16(), value.ok()));
    }
    let value = value.map_err(|error| {
        ControlPlaneRequestError::Contract(format!(
            "control plane returned invalid proof submission JSON: {error}"
        ))
    })?;
    serde_json::from_value(value).map_err(|error| {
        ControlPlaneRequestError::Contract(format!(
            "invalid proof submission response contract: {error}"
        ))
    })
}

fn response_error(status: u16, value: Option<serde_json::Value>) -> ControlPlaneRequestError {
    let value = value.unwrap_or(serde_json::Value::Null);
    ControlPlaneRequestError::Rejected {
        status,
        code: value["error"]["code"]
            .as_str()
            .unwrap_or("remote_error")
            .to_string(),
        message: value["error"]["message"]
            .as_str()
            .unwrap_or("control plane rejected request")
            .to_string(),
    }
}

async fn wait_for_poll_or_shutdown(shutdown: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::time::sleep(PROOF_POLL_INTERVAL) => {}
        _ = shutdown.changed() => {}
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

fn log_proof_event(event: &str, challenge_id: Option<&str>, error: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "event": event,
            "challenge_id": challenge_id,
            "error": error,
        })
    );
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| format!("invalid proof timestamp: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_proof_set_rejects_unknown_requirements() {
        let supported = PROOF_CAPABILITY_REQUIRED_PROOFS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(supported.contains("cuda_runtime"));
        assert!(!supported.contains("self_reported_score"));
    }

    #[test]
    fn telemetry_window_requires_current_proof_gpu_and_time() {
        let started_at = Utc::now();
        let mut stale_sample = sample("GPU-00112233-4455-6677-8899-aabbccddeeff");
        stale_sample.observed_at = (started_at - chrono::Duration::seconds(10)).to_rfc3339();
        let window = ProofTelemetryWindow {
            batch_hash: "sha256:batch".to_string(),
            samples: vec![
                stale_sample,
                sample("GPU-00112233-4455-6677-8899-aabbccddeeff"),
            ],
        };
        assert!(
            validate_telemetry_window(
                &window,
                "GPU-00112233-4455-6677-8899-aabbccddeeff",
                started_at,
            )
            .is_ok()
        );
        assert!(validate_telemetry_window(&window, "GPU-other", started_at).is_err());
    }
    #[test]
    fn malformed_or_expired_challenge_deadlines_still_throttle_polling() {
        let recorded_at = Utc::now();
        let expected =
            recorded_at + chrono::Duration::seconds(PROOF_POLL_INTERVAL.as_secs() as i64);
        assert_eq!(suppression_deadline("invalid", recorded_at), expected);
        assert_eq!(
            suppression_deadline(
                &(recorded_at - chrono::Duration::seconds(1)).to_rfc3339(),
                recorded_at,
            ),
            expected
        );
    }

    static COOPERATIVE_EXECUTOR_PASSED_GATE: AtomicBool = AtomicBool::new(false);

    #[tokio::test]
    async fn active_proof_execution_stops_cooperatively_on_shutdown() {
        COOPERATIVE_EXECUTOR_PASSED_GATE.store(false, Ordering::SeqCst);
        let (telemetry_tx, mut telemetry_rx) = mpsc::channel::<ProofTelemetryRequest>(1);
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let responder = tokio::spawn(async move {
            let request = telemetry_rx
                .recv()
                .await
                .expect("proof telemetry request was not sent");
            request
                .response
                .send(Ok(ProofTelemetryWindow {
                    batch_hash: "sha256:cooperative-shutdown".to_string(),
                    samples: vec![sample("GPU-cooperative-shutdown")],
                }))
                .expect("proof telemetry response was dropped");
            tokio::time::timeout(Duration::from_secs(1), async {
                while !COOPERATIVE_EXECUTOR_PASSED_GATE.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("proof executor did not pass the telemetry gate");
            shutdown_tx
                .send(true)
                .expect("proof shutdown receiver was dropped");
        });

        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            execute_and_submit_challenge(
                proof_challenge(),
                &telemetry_tx,
                cooperative_shutdown_executor,
                &mut shutdown_rx,
            ),
        )
        .await
        .expect("proof shutdown exceeded the test deadline")
        .expect("proof shutdown returned an execution failure");

        assert!(outcome.is_none());
        responder.await.unwrap();
    }

    fn cooperative_shutdown_executor(
        mut request: ProofExecutionRequest,
    ) -> Result<ProofExecution, String> {
        request.hold_residency_for_telemetry("GPU-cooperative-shutdown".to_string())?;
        COOPERATIVE_EXECUTOR_PASSED_GATE.store(true, Ordering::SeqCst);
        while !request.cancellation_requested() {
            std::thread::sleep(Duration::from_millis(1));
        }
        request.ensure_not_cancelled()?;
        Err("proof executor ignored cancellation".to_string())
    }

    fn proof_challenge() -> ProofCapabilityChallenge {
        ProofCapabilityChallenge {
            schema_version: PROOF_CHALLENGE_SCHEMA_VERSION.to_string(),
            challenge_id: "challenge_cooperative_shutdown".to_string(),
            nonce: "nonce_cooperative_shutdown".to_string(),
            provider_id: "provider_test".to_string(),
            device_id: "device_test".to_string(),
            session_id: "session_test".to_string(),
            profile_version: "profile_test".to_string(),
            required_fingerprint: "sha256:fingerprint".to_string(),
            required_gpu_uuid: Some("GPU-cooperative-shutdown".to_string()),
            required_backend: "cuda".to_string(),
            model_artifact_hash: "sha256:model".to_string(),
            prompt_seed: "prompt_seed".to_string(),
            required_proofs: PROOF_CAPABILITY_REQUIRED_PROOFS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            min_tokens_per_second: 0.0,
            max_ttft_ms: 0,
            issued_at: (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339(),
            expires_at: (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
        }
    }
    fn sample(gpu_uuid: &str) -> GpuTelemetrySample {
        GpuTelemetrySample {
            sample_sequence: 1,
            observed_at: Utc::now().to_rfc3339(),
            gpu_uuid: gpu_uuid.to_string(),
            gpu_name: "NVIDIA Test".to_string(),
            pci_bus_id: "00000000:01:00.0".to_string(),
            pci_vendor_id: Some("10de".to_string()),
            pci_device_id: Some("0000".to_string()),
            compute_capability: Some("8.9".to_string()),
            driver_version: "576.80".to_string(),
            cuda_driver_version: Some("12.9".to_string()),
            cuda_runtime_version: Some("12.8".to_string()),
            vram_total_mib: 8192,
            vram_used_mib: Some(512),
            vram_free_mib: Some(7680),
            gpu_utilization_percent: Some(0.0),
            memory_utilization_percent: Some(0.0),
            temperature_celsius: Some(40.0),
            power_draw_watts: Some(20.0),
            power_limit_watts: Some(200.0),
            graphics_clock_mhz: Some(210),
            sm_clock_mhz: Some(210),
            memory_clock_mhz: Some(405),
            performance_state: Some("P8".to_string()),
            throttle_reasons: vec![],
            ecc_corrected_errors: None,
            ecc_uncorrected_errors: None,
            processes: vec![],
            container_id: None,
            job_id: None,
        }
    }
}
