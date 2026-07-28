use crate::exit_status::AgentExitError;
use crate::lifecycle::{AgentLifecyclePhase, LifecycleReporter};
use crate::remote_enrollment::{
    ControlPlaneRequestError, join_url, post_json_checked, refresh_credential,
    refresh_credential_checked,
};
use crate::remote_proof::{
    ProofExecutor, ProofTelemetryRequest, ProofTelemetryWindow, execute_remote_proof, run_worker,
};
use crate::{AgentStateLock, AgentStateLockOperation};
use burd_bench::{ProviderRegistrationPayload, build_registration_payload};
use burd_hardware::{NvidiaTelemetryCollection, collect_nvidia_telemetry};
use burd_protocol::{
    ClientControlMessage, GpuTelemetrySample, HeartbeatPayload, RemoteEnrollmentState,
    RemoteSessionRecord, RemoteSessionResume, RemoteSessionState, RemoteSessionStateStatus,
    ServerControlMessage, SignedTelemetryBatch, StartRemoteSessionRequest,
    StartRemoteSessionResponse, TELEMETRY_CANONICALIZATION_VERSION, TELEMETRY_SCHEMA_VERSION,
    TelemetryBatchPayload, TelemetryBatchReceipt, clear_remote_session, load_identity,
    load_private_key, load_remote_enrollment, load_remote_session, load_remote_session_optional,
    save_remote_session, show_remote_session, sign_message, telemetry_batch_hash,
    telemetry_batch_signature_message, update_remote_session_sequence,
    update_remote_telemetry_sequence,
};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};

const SESSION_OPERATION_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

pub fn connect(
    agent_version: &str,
    max_reconnect_delay_seconds: u64,
    telemetry: bool,
    telemetry_batch_samples: usize,
    proofs: bool,
) -> Result<Option<RemoteSessionStateStatus>, AgentExitError> {
    validate_telemetry_batch_samples(telemetry_batch_samples)
        .map_err(|error| AgentExitError::invalid_invocation("telemetry_config", error))?;
    let _instance_lock = AgentStateLock::acquire(AgentStateLockOperation::RemoteSessionConnect)
        .map_err(|error| AgentExitError::local_state("state_lock", error))?;
    let lifecycle = LifecycleReporter::start()
        .map_err(|error| AgentExitError::local_state("lifecycle_state", error))?;
    let result: Result<(), AgentExitError> = (|| {
        let identity =
            load_identity().map_err(|error| AgentExitError::local_state("local_state", error))?;
        let telemetry_enabled = proofs || telemetry || identity.telemetry_enabled;
        let proof_agent_version = proofs.then(|| agent_version.to_string());
        let retry_seed = stable_retry_seed(&identity.machine_id);
        let runtime = tokio::runtime::Runtime::new().map_err(|error| {
            AgentExitError::internal(
                "session_runtime",
                format!("failed to start remote session runtime: {error}"),
            )
        })?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let runtime_lifecycle = lifecycle.clone();
        runtime.block_on(async move {
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown_tx.send(true);
            });
            run_control_and_proof(
                agent_version.to_string(),
                max_reconnect_delay_seconds.max(1),
                retry_seed,
                telemetry_enabled,
                telemetry_batch_samples,
                collect_nvidia_telemetry,
                proof_agent_version,
                execute_remote_proof,
                prepare_connection,
                runtime_lifecycle,
                shutdown_rx,
            )
            .await
        })
    })();
    complete_lifecycle(&lifecycle, &result)?;
    result?;
    let persisted = load_remote_session_optional()
        .map_err(|error| AgentExitError::local_state("local_state", error))?;
    if persisted.is_none() {
        return Ok(None);
    }
    show_remote_session()
        .map(Some)
        .map_err(|error| AgentExitError::local_state("local_state", error))
}

fn complete_lifecycle(
    lifecycle: &LifecycleReporter,
    result: &Result<(), AgentExitError>,
) -> Result<(), AgentExitError> {
    match result {
        Ok(()) => lifecycle
            .transition(AgentLifecyclePhase::Stopped, None)
            .map_err(|error| AgentExitError::local_state("lifecycle_state", error)),
        Err(_)
            if lifecycle
                .phase()
                .map_err(|error| AgentExitError::local_state("lifecycle_state", error))?
                == AgentLifecyclePhase::TerminalFailure =>
        {
            Ok(())
        }
        Err(error) => lifecycle
            .transition(
                AgentLifecyclePhase::TerminalFailure,
                Some(error.failure_kind()),
            )
            .map_err(|error| AgentExitError::local_state("lifecycle_state", error)),
    }
}

#[cfg(feature = "integration-test-support")]
async fn complete_lifecycle_async(
    lifecycle: &LifecycleReporter,
    result: &Result<(), AgentExitError>,
) -> Result<(), AgentExitError> {
    match result {
        Ok(()) => transition_lifecycle(lifecycle, AgentLifecyclePhase::Stopped, None)
            .await
            .map_err(lifecycle_exit_error),
        Err(_)
            if lifecycle
                .phase()
                .map_err(|error| AgentExitError::local_state("lifecycle_state", error))?
                == AgentLifecyclePhase::TerminalFailure =>
        {
            Ok(())
        }
        Err(error) => transition_lifecycle(
            lifecycle,
            AgentLifecyclePhase::TerminalFailure,
            Some(error.failure_kind()),
        )
        .await
        .map_err(lifecycle_exit_error),
    }
}

async fn transition_lifecycle(
    lifecycle: &LifecycleReporter,
    phase: AgentLifecyclePhase,
    failure_kind: Option<&'static str>,
) -> Result<(), String> {
    let lifecycle = lifecycle.clone();
    tokio::task::spawn_blocking(move || lifecycle.transition(phase, failure_kind))
        .await
        .map_err(|error| format!("Agent lifecycle transition task failed: {error}"))?
}

fn lifecycle_exit_error(error: String) -> AgentExitError {
    AgentExitError::local_state("lifecycle_state", error)
}

#[cfg(feature = "integration-test-support")]
pub async fn connect_until_shutdown(
    agent_version: &str,
    max_reconnect_delay_seconds: u64,
    telemetry: bool,
    telemetry_batch_samples: usize,
    shutdown: watch::Receiver<bool>,
) -> Result<RemoteSessionStateStatus, AgentExitError> {
    connect_until_shutdown_with_telemetry_collector(
        agent_version,
        max_reconnect_delay_seconds,
        telemetry,
        telemetry_batch_samples,
        collect_nvidia_telemetry,
        shutdown,
    )
    .await
}

#[cfg(feature = "integration-test-support")]
pub async fn connect_until_shutdown_with_telemetry_collector(
    agent_version: &str,
    max_reconnect_delay_seconds: u64,
    telemetry: bool,
    telemetry_batch_samples: usize,
    telemetry_collector: fn(u64) -> Result<NvidiaTelemetryCollection, String>,
    shutdown: watch::Receiver<bool>,
) -> Result<RemoteSessionStateStatus, AgentExitError> {
    validate_telemetry_batch_samples(telemetry_batch_samples)
        .map_err(|error| AgentExitError::invalid_invocation("telemetry_config", error))?;
    let _instance_lock = AgentStateLock::acquire(AgentStateLockOperation::RemoteSessionConnect)
        .map_err(|error| AgentExitError::local_state("state_lock", error))?;
    let lifecycle = tokio::task::spawn_blocking(LifecycleReporter::start)
        .await
        .map_err(|error| {
            AgentExitError::internal(
                "lifecycle_task",
                format!("failed to start Agent lifecycle task: {error}"),
            )
        })?
        .map_err(|error| AgentExitError::local_state("lifecycle_state", error))?;
    let result = async {
        let identity = tokio::task::spawn_blocking(load_identity)
            .await
            .map_err(|error| {
                AgentExitError::internal(
                    "identity_task",
                    format!("failed to load identity task: {error}"),
                )
            })?
            .map_err(|error| AgentExitError::local_state("local_state", error))?;
        let telemetry_enabled = telemetry || identity.telemetry_enabled;
        let retry_seed = stable_retry_seed(&identity.machine_id);
        run_control_and_proof(
            agent_version.to_string(),
            max_reconnect_delay_seconds.max(1),
            retry_seed,
            telemetry_enabled,
            telemetry_batch_samples,
            telemetry_collector,
            None,
            execute_remote_proof,
            prepare_connection,
            lifecycle.clone(),
            shutdown,
        )
        .await
    }
    .await;
    complete_lifecycle_async(&lifecycle, &result).await?;
    result?;
    tokio::task::spawn_blocking(show_remote_session)
        .await
        .map_err(|error| {
            AgentExitError::internal(
                "session_status_task",
                format!("failed to load remote session status task: {error}"),
            )
        })?
        .map_err(|error| AgentExitError::local_state("local_state", error))
}

#[cfg(feature = "integration-test-support")]
#[allow(clippy::too_many_arguments)]
pub async fn connect_until_shutdown_with_test_runtime(
    agent_version: &str,
    max_reconnect_delay_seconds: u64,
    telemetry_batch_samples: usize,
    telemetry_collector: fn(u64) -> Result<NvidiaTelemetryCollection, String>,
    proof_executor: ProofExecutor,
    shutdown: watch::Receiver<bool>,
) -> Result<RemoteSessionStateStatus, AgentExitError> {
    validate_telemetry_batch_samples(telemetry_batch_samples)
        .map_err(|error| AgentExitError::invalid_invocation("telemetry_config", error))?;
    let _instance_lock = AgentStateLock::acquire(AgentStateLockOperation::RemoteSessionConnect)
        .map_err(|error| AgentExitError::local_state("state_lock", error))?;
    let lifecycle = tokio::task::spawn_blocking(LifecycleReporter::start)
        .await
        .map_err(|error| {
            AgentExitError::internal(
                "lifecycle_task",
                format!("failed to start Agent lifecycle task: {error}"),
            )
        })?
        .map_err(|error| AgentExitError::local_state("lifecycle_state", error))?;
    let result = async {
        let identity = tokio::task::spawn_blocking(load_identity)
            .await
            .map_err(|error| {
                AgentExitError::internal(
                    "identity_task",
                    format!("failed to load identity task: {error}"),
                )
            })?
            .map_err(|error| AgentExitError::local_state("local_state", error))?;
        let retry_seed = stable_retry_seed(&identity.machine_id);
        run_control_and_proof(
            agent_version.to_string(),
            max_reconnect_delay_seconds.max(1),
            retry_seed,
            true,
            telemetry_batch_samples,
            telemetry_collector,
            Some(agent_version.to_string()),
            proof_executor,
            prepare_connection,
            lifecycle.clone(),
            shutdown,
        )
        .await
    }
    .await;
    complete_lifecycle_async(&lifecycle, &result).await?;
    result?;
    tokio::task::spawn_blocking(show_remote_session)
        .await
        .map_err(|error| {
            AgentExitError::internal(
                "session_status_task",
                format!("failed to load remote session status task: {error}"),
            )
        })?
        .map_err(|error| AgentExitError::local_state("local_state", error))
}

#[allow(clippy::too_many_arguments)]
async fn run_control_and_proof(
    agent_version: String,
    max_reconnect_delay_seconds: u64,
    retry_seed: u64,
    telemetry_enabled: bool,
    telemetry_batch_samples: usize,
    telemetry_collector: fn(u64) -> Result<NvidiaTelemetryCollection, String>,
    proof_agent_version: Option<String>,
    proof_executor: ProofExecutor,
    preparation_executor: PreparationExecutor,
    lifecycle: LifecycleReporter,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), AgentExitError> {
    let (proof_telemetry_tx, mut proof_telemetry_rx) = mpsc::channel(1);
    let (session_shutdown_tx, session_shutdown_rx) = watch::channel(false);
    let proof_worker = proof_agent_version.map(|proof_agent_version| {
        tokio::spawn(run_worker(
            proof_agent_version,
            proof_telemetry_tx,
            proof_executor,
            session_shutdown_rx.clone(),
        ))
    });
    let control = run_control_loop(
        ControlLoopRuntime {
            agent_version,
            max_reconnect_delay_seconds,
            retry_seed,
            telemetry: TelemetryRuntime {
                enabled: telemetry_enabled,
                batch_samples: telemetry_batch_samples,
                collector: telemetry_collector,
            },
            preparation_executor,
            lifecycle: lifecycle.clone(),
        },
        &mut proof_telemetry_rx,
        session_shutdown_rx,
    );
    tokio::pin!(control);

    match proof_worker {
        Some(mut worker) => {
            tokio::select! {
                _ = wait_for_shutdown(&mut shutdown) => {
                    let lifecycle_result = transition_lifecycle(
                        &lifecycle,
                        AgentLifecyclePhase::Stopping,
                        None,
                    ).await;
                    let _ = session_shutdown_tx.send(true);
                    let result = control.await;
                    lifecycle_result.map_err(lifecycle_exit_error)?;
                    finish_proof_worker(worker, result).await
                }
                result = &mut control => {
                    let _ = session_shutdown_tx.send(true);
                    finish_proof_worker(worker, result).await
                }
                worker_result = &mut worker => {
                    let _ = session_shutdown_tx.send(true);
                    let control_result = control.await;
                    if let Err(error) = control_result {
                        log_proof_worker_shutdown_error(error.failure_kind());
                    }
                    Err(unexpected_proof_worker_exit(worker_result))
                }
            }
        }
        None => {
            tokio::select! {
                _ = wait_for_shutdown(&mut shutdown) => {
                    let lifecycle_result = transition_lifecycle(
                        &lifecycle,
                        AgentLifecyclePhase::Stopping,
                        None,
                    ).await;
                    let _ = session_shutdown_tx.send(true);
                    let result = control.await;
                    lifecycle_result.map_err(lifecycle_exit_error)?;
                    result
                }
                result = &mut control => result,
            }
        }
    }
}

async fn finish_proof_worker(
    worker: tokio::task::JoinHandle<Result<(), String>>,
    control_result: Result<(), AgentExitError>,
) -> Result<(), AgentExitError> {
    match worker.await {
        Ok(Ok(())) => control_result,
        Ok(Err(error)) if control_result.is_ok() => {
            Err(AgentExitError::internal("proof_worker", error))
        }
        Ok(Err(_error)) => {
            log_proof_worker_shutdown_error("proof_worker");
            control_result
        }
        Err(error) if control_result.is_ok() => Err(AgentExitError::internal(
            "proof_worker",
            format!("remote proof worker failed: {error}"),
        )),
        Err(_error) => {
            log_proof_worker_shutdown_error("proof_worker");
            control_result
        }
    }
}

fn unexpected_proof_worker_exit(
    result: Result<Result<(), String>, tokio::task::JoinError>,
) -> AgentExitError {
    let detail = match result {
        Ok(Ok(())) => "remote proof worker stopped unexpectedly".to_string(),
        Ok(Err(error)) => format!("remote proof worker failed: {error}"),
        Err(error) => format!("remote proof worker task failed: {error}"),
    };
    AgentExitError::internal("proof_worker", detail)
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
    cancellation: &SessionOperationCancellation,
) -> Result<RemoteSessionStateStatus, ReconnectFailure> {
    ensure_session_not_cancelled(cancellation)?;
    let enrollment = load_remote_enrollment().map_err(|error| {
        ReconnectFailure::terminal("local_state", format!("failed to load enrollment: {error}"))
    })?;
    ensure_session_not_cancelled(cancellation)?;
    let mut persisted = load_remote_session_optional().map_err(|error| {
        ReconnectFailure::terminal(
            "local_state",
            format!("failed to load remote session: {error}"),
        )
    })?;
    if persisted.as_ref().is_some_and(|state| {
        !session_belongs_to_control_plane(state, &enrollment.control_plane_url)
    }) {
        clear_remote_session().map_err(|error| {
            ReconnectFailure::terminal(
                "local_state",
                format!("failed to clear remote session from another control plane: {error}"),
            )
        })?;
        persisted = None;
    }
    ensure_session_not_cancelled(cancellation)?;
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
    ensure_session_not_cancelled(cancellation)?;
    let result: Result<StartRemoteSessionResponse, ControlPlaneRequestError> = post_json_checked(
        &join_url(&enrollment.control_plane_url, "/v1/sessions"),
        &request,
        Some(&enrollment.credential),
    );
    // Persist a successful backend side effect before the caller honors shutdown.
    let response = match result {
        Ok(response) => response,
        Err(error) if persisted.is_some() && persisted_session_should_restart(&error) => {
            clear_remote_session().map_err(|error| {
                ReconnectFailure::terminal(
                    "local_state",
                    format!("failed to clear expired remote session: {error}"),
                )
            })?;
            return start_or_resume(agent_version, registration, cancellation);
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

fn ensure_session_not_cancelled(
    cancellation: &SessionOperationCancellation,
) -> Result<(), ReconnectFailure> {
    cancellation
        .ensure_not_cancelled()
        .map_err(|error| ReconnectFailure::retry("preparation_cancelled", error))
}

fn session_belongs_to_control_plane(session: &RemoteSessionState, control_plane_url: &str) -> bool {
    session.control_plane_url.trim_end_matches('/') == control_plane_url.trim_end_matches('/')
}

type PreparationExecutor = fn(PreparationRequest) -> Result<PreparedConnection, ReconnectFailure>;

struct PreparedConnection {
    enrollment: RemoteEnrollmentState,
    session: RemoteSessionState,
    hardware_fingerprint: String,
}

struct PreparationRequest {
    agent_version: String,
    cancellation: SessionOperationCancellation,
}

impl PreparationRequest {
    fn ensure_not_cancelled(&self) -> Result<(), ReconnectFailure> {
        self.cancellation
            .ensure_not_cancelled()
            .map_err(|error| ReconnectFailure::retry("preparation_cancelled", error))
    }
}

#[derive(Debug, Clone, Default)]
struct SessionOperationCancellation {
    requested: Arc<AtomicBool>,
}

impl SessionOperationCancellation {
    fn cancel(&self) {
        self.requested.store(true, Ordering::Release);
    }

    fn ensure_not_cancelled(&self) -> Result<(), String> {
        if self.requested.load(Ordering::Acquire) {
            Err("remote session operation cancelled by Agent shutdown".to_string())
        } else {
            Ok(())
        }
    }
}

fn prepare_connection(request: PreparationRequest) -> Result<PreparedConnection, ReconnectFailure> {
    request.ensure_not_cancelled()?;
    let enrollment = ensure_credential_fresh(&request.cancellation)?;
    request.ensure_not_cancelled()?;
    let registration = build_registration_payload(&request.agent_version);
    request.ensure_not_cancelled()?;
    let hardware_fingerprint = registration.hardware_fingerprint.clone();
    start_or_resume(&request.agent_version, &registration, &request.cancellation)?;
    request.ensure_not_cancelled()?;
    let session = load_remote_session().map_err(|error| {
        ReconnectFailure::terminal(
            "local_state",
            format!("failed to load remote session: {error}"),
        )
    })?;
    Ok(PreparedConnection {
        enrollment,
        session,
        hardware_fingerprint,
    })
}

async fn run_preparation_or_shutdown(
    agent_version: String,
    executor: PreparationExecutor,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Option<PreparedConnection>, ReconnectFailure> {
    let cancellation = SessionOperationCancellation::default();
    let request = PreparationRequest {
        agent_version,
        cancellation: cancellation.clone(),
    };
    let mut task = tokio::task::spawn_blocking(move || executor(request));
    tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => {
            stop_session_operation(
                &cancellation,
                &mut task,
                "connection_preparation",
            ).await;
            Ok(None)
        }
        result = &mut task => {
            result
                .map_err(|error| {
                    ReconnectFailure::terminal(
                        "internal_task",
                        format!("remote session preparation task failed: {error}"),
                    )
                })?
                .map(Some)
        }
    }
}

async fn stop_session_operation<T, E>(
    cancellation: &SessionOperationCancellation,
    task: &mut tokio::task::JoinHandle<Result<T, E>>,
    operation: &'static str,
) where
    T: Send + 'static,
    E: Send + 'static,
{
    cancellation.cancel();
    if tokio::time::timeout(SESSION_OPERATION_SHUTDOWN_GRACE, &mut *task)
        .await
        .is_err()
    {
        task.abort();
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "remote_session_operation_shutdown_grace_exceeded",
                "operation": operation,
                "grace_seconds": SESSION_OPERATION_SHUTDOWN_GRACE.as_secs(),
            })
        );
    }
}

async fn refresh_credential_or_shutdown(
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Option<RemoteEnrollmentState>, String> {
    let cancellation = SessionOperationCancellation::default();
    let worker_cancellation = cancellation.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        worker_cancellation.ensure_not_cancelled()?;
        refresh_credential()?;
        worker_cancellation.ensure_not_cancelled()?;
        load_remote_enrollment()
    });
    tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => {
            stop_session_operation(
                &cancellation,
                &mut task,
                "credential_refresh",
            ).await;
            Ok(None)
        }
        result = &mut task => {
            result
                .map_err(|error| format!("credential refresh task failed: {error}"))?
                .map(Some)
        }
    }
}

struct ControlLoopRuntime {
    agent_version: String,
    max_reconnect_delay_seconds: u64,
    retry_seed: u64,
    telemetry: TelemetryRuntime,
    preparation_executor: PreparationExecutor,
    lifecycle: LifecycleReporter,
}

async fn run_control_loop(
    runtime: ControlLoopRuntime,
    proof_telemetry: &mut mpsc::Receiver<ProofTelemetryRequest>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), AgentExitError> {
    let mut retry_policy =
        ReconnectPolicy::new(runtime.max_reconnect_delay_seconds, runtime.retry_seed);
    loop {
        if shutdown_requested(&shutdown) {
            return Ok(());
        }
        transition_lifecycle(&runtime.lifecycle, AgentLifecyclePhase::Connecting, None)
            .await
            .map_err(lifecycle_exit_error)?;
        match attempt_connection(
            runtime.agent_version.clone(),
            runtime.telemetry,
            runtime.preparation_executor,
            &runtime.lifecycle,
            proof_telemetry,
            &mut shutdown,
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(failure) => {
                if failure.disposition == ReconnectDisposition::Stop {
                    transition_lifecycle(
                        &runtime.lifecycle,
                        AgentLifecyclePhase::TerminalFailure,
                        Some(failure.kind),
                    )
                    .await
                    .map_err(lifecycle_exit_error)?;
                    return Err(AgentExitError::from_failure_kind(
                        failure.kind,
                        failure.message,
                    ));
                }
                transition_lifecycle(
                    &runtime.lifecycle,
                    AgentLifecyclePhase::Degraded,
                    Some(failure.kind),
                )
                .await
                .map_err(lifecycle_exit_error)?;
                if failure.disposition == ReconnectDisposition::RestartSession
                    && let Err(error) = clear_remote_session()
                {
                    let message = format!("failed to clear remote session before restart: {error}");
                    transition_lifecycle(
                        &runtime.lifecycle,
                        AgentLifecyclePhase::TerminalFailure,
                        Some("local_state"),
                    )
                    .await
                    .map_err(lifecycle_exit_error)?;
                    return Err(AgentExitError::local_state("local_state", message));
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
    telemetry: TelemetryRuntime,
    preparation_executor: PreparationExecutor,
    lifecycle: &LifecycleReporter,
    proof_telemetry: &mut mpsc::Receiver<ProofTelemetryRequest>,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), ReconnectFailure> {
    if shutdown_requested(shutdown) {
        return Ok(());
    }
    let Some(prepared) =
        run_preparation_or_shutdown(agent_version, preparation_executor, shutdown).await?
    else {
        return Ok(());
    };
    let mut connection_was_stable = false;
    match run_one_connection(
        prepared,
        telemetry,
        &mut connection_was_stable,
        lifecycle,
        proof_telemetry,
        shutdown,
    )
    .await
    {
        Ok(ConnectionOutcome::Stopped) => Ok(()),
        Ok(ConnectionOutcome::Terminal { kind, reason }) => {
            Err(ReconnectFailure::terminal(kind, reason)
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
    Terminal { kind: &'static str, reason: String },
    RestartSession(String),
}

impl ConnectionOutcome {
    fn terminal(kind: &'static str, reason: &str) -> Self {
        Self::Terminal {
            kind,
            reason: reason.to_string(),
        }
    }
}

fn websocket_http_outcome(status: u16) -> Option<ConnectionOutcome> {
    match status {
        400 => Some(ConnectionOutcome::terminal(
            "control_plane_contract",
            "control plane rejected the WebSocket request contract",
        )),
        401 => Some(ConnectionOutcome::terminal(
            "unauthorized",
            "control channel credential is invalid or expired",
        )),
        403 => Some(ConnectionOutcome::terminal(
            "session_revoked",
            "control channel device or session has been revoked",
        )),
        404 | 410 => Some(ConnectionOutcome::RestartSession(
            "remote session is missing or expired".to_string(),
        )),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct TelemetryRuntime {
    enabled: bool,
    batch_samples: usize,
    collector: fn(u64) -> Result<NvidiaTelemetryCollection, String>,
}

async fn run_one_connection(
    prepared: PreparedConnection,
    telemetry: TelemetryRuntime,
    connection_was_stable: &mut bool,
    lifecycle: &LifecycleReporter,
    proof_telemetry: &mut mpsc::Receiver<ProofTelemetryRequest>,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<ConnectionOutcome, String> {
    let PreparedConnection {
        mut enrollment,
        session,
        hardware_fingerprint,
    } = prepared;
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
    let connection = tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => return Ok(ConnectionOutcome::Stopped),
        result = connect_async(request) => result,
    };
    let (mut socket, _) = match connection {
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

    let ready = tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => {
            let _ = socket.close(None).await;
            return Ok(ConnectionOutcome::Stopped);
        }
        result = receive_server_message(&mut socket, Duration::from_secs(10)) => result?,
    };
    if ready.message_type != "session_ready" {
        return Err(format!(
            "control channel expected session_ready, received {}",
            ready.message_type
        ));
    }
    transition_lifecycle(lifecycle, AgentLifecyclePhase::Online, None).await?;
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
    let mut telemetry_active = telemetry.enabled;
    let mut proof_channel_open = true;
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        let action = tokio::select! {
            _ = wait_for_shutdown(shutdown) => {
                let _ = socket.close(None).await;
                return Ok(ConnectionOutcome::Stopped);
            }
            request = proof_telemetry.recv(), if proof_channel_open => {
                match request {
                    Some(request) => ConnectionAction::ProofTelemetry(request),
                    None => ConnectionAction::ProofChannelClosed,
                }
            }
            _ = heartbeat.tick() => ConnectionAction::Heartbeat,
        };

        match action {
            ConnectionAction::ProofChannelClosed => {
                proof_channel_open = false;
                continue;
            }
            ConnectionAction::Heartbeat => {
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
                        return Ok(ConnectionOutcome::terminal(
                            "session_revoked",
                            "remote session was revoked by the control plane",
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
                    match collect_and_submit_telemetry(
                        &mut socket,
                        &enrollment,
                        &session,
                        &hardware_fingerprint,
                        telemetry,
                        &mut sequence,
                        &mut telemetry_sequence,
                        &mut telemetry_samples,
                        false,
                        response_timeout,
                    )
                    .await?
                    {
                        TelemetryOutcome::Buffered | TelemetryOutcome::Accepted(_) => {
                            telemetry_unavailable_logged = false;
                        }
                        TelemetryOutcome::Unavailable(error) => {
                            if !telemetry_unavailable_logged {
                                log_telemetry_unavailable(&error);
                                telemetry_unavailable_logged = true;
                            }
                        }
                        TelemetryOutcome::Rejected(error) => {
                            log_telemetry_rejected(&error);
                            telemetry_active = false;
                        }
                        TelemetryOutcome::SessionRevoked => {
                            return Ok(ConnectionOutcome::terminal(
                                "session_revoked",
                                "remote session was revoked by the control plane",
                            ));
                        }
                    }
                }
            }
            ConnectionAction::ProofTelemetry(request) => {
                if !telemetry_active {
                    let _ = request.response.send(Err(
                        "signed GPU telemetry is unavailable for remote proof".to_string(),
                    ));
                    continue;
                }
                let outcome = collect_and_submit_telemetry(
                    &mut socket,
                    &enrollment,
                    &session,
                    &hardware_fingerprint,
                    telemetry,
                    &mut sequence,
                    &mut telemetry_sequence,
                    &mut telemetry_samples,
                    true,
                    response_timeout,
                )
                .await;
                match outcome {
                    Ok(TelemetryOutcome::Accepted(window)) => {
                        telemetry_unavailable_logged = false;
                        let includes_gpu = window.samples.iter().any(|sample| {
                            sample
                                .gpu_uuid
                                .eq_ignore_ascii_case(&request.required_gpu_uuid)
                        });
                        let result = if includes_gpu {
                            Ok(window)
                        } else {
                            Err(format!(
                                "signed telemetry did not include proof GPU {}",
                                request.required_gpu_uuid
                            ))
                        };
                        let _ = request.response.send(result);
                    }
                    Ok(TelemetryOutcome::Unavailable(error)) => {
                        if !telemetry_unavailable_logged {
                            log_telemetry_unavailable(&error);
                            telemetry_unavailable_logged = true;
                        }
                        let _ = request.response.send(Err(error));
                    }
                    Ok(TelemetryOutcome::Rejected(error)) => {
                        log_telemetry_rejected(&error);
                        telemetry_active = false;
                        let _ = request.response.send(Err(error));
                    }
                    Ok(TelemetryOutcome::SessionRevoked) => {
                        let _ = request
                            .response
                            .send(Err("remote session was revoked".to_string()));
                        return Ok(ConnectionOutcome::terminal(
                            "session_revoked",
                            "remote session was revoked by the control plane",
                        ));
                    }
                    Ok(TelemetryOutcome::Buffered) => {
                        let _ = request.response.send(Err(
                            "forced proof telemetry was buffered without submission".to_string(),
                        ));
                    }
                    Err(error) => {
                        let _ = request.response.send(Err(error.clone()));
                        return Err(error);
                    }
                }
            }
        }

        if credential_refresh_due(&enrollment)? {
            let Some(refreshed) = refresh_credential_or_shutdown(shutdown).await? else {
                return Ok(ConnectionOutcome::Stopped);
            };
            enrollment = refreshed;
        }
    }
}
#[derive(Debug)]
enum ConnectionAction {
    Heartbeat,
    ProofTelemetry(ProofTelemetryRequest),
    ProofChannelClosed,
}

#[derive(Debug)]
enum TelemetryOutcome {
    Buffered,
    Accepted(ProofTelemetryWindow),
    Unavailable(String),
    Rejected(String),
    SessionRevoked,
}

#[allow(clippy::too_many_arguments)]
async fn collect_and_submit_telemetry<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    enrollment: &RemoteEnrollmentState,
    session: &RemoteSessionState,
    hardware_fingerprint: &str,
    telemetry: TelemetryRuntime,
    sequence: &mut u64,
    telemetry_sequence: &mut u64,
    telemetry_samples: &mut Vec<GpuTelemetrySample>,
    force_submit: bool,
    response_timeout: Duration,
) -> Result<TelemetryOutcome, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if force_submit {
        telemetry_samples.clear();
    }
    let next_sample_sequence = telemetry_sequence
        .checked_add(telemetry_samples.len() as u64 + 1)
        .ok_or_else(|| "GPU telemetry sequence overflow".to_string())?;
    let collection =
        match tokio::task::spawn_blocking(move || (telemetry.collector)(next_sample_sequence))
            .await
            .map_err(|error| format!("GPU telemetry collection task failed: {error}"))?
        {
            Ok(collection) => collection,
            Err(error) => return Ok(TelemetryOutcome::Unavailable(error)),
        };
    for warning in &collection.warnings {
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
    if !force_submit && telemetry_samples.len() < telemetry.batch_samples {
        return Ok(TelemetryOutcome::Buffered);
    }

    *sequence = sequence.saturating_add(1);
    let signed = build_signed_telemetry_batch(
        enrollment,
        session,
        hardware_fingerprint,
        *sequence,
        collection.collector,
        telemetry_samples,
    )?;
    let batch_hash = signed.batch_hash.clone();
    let submitted_samples = signed.payload.samples.clone();
    let telemetry_message = ClientControlMessage {
        session_id: session.session_id.clone(),
        device_id: enrollment.device_id.clone(),
        sequence: *sequence,
        sent_at: Utc::now().to_rfc3339(),
        message_type: "telemetry_batch".to_string(),
        payload: serde_json::to_value(signed)
            .map_err(|error| format!("failed to serialize telemetry batch: {error}"))?,
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&telemetry_message)
                .map_err(|error| format!("failed to serialize telemetry message: {error}"))?
                .into(),
        ))
        .await
        .map_err(|error| format!("failed to send telemetry batch: {error}"))?;
    let response = receive_server_message(socket, response_timeout).await?;
    match response.message_type.as_str() {
        "telemetry_ack" => {
            let receipt: TelemetryBatchReceipt = serde_json::from_value(response.payload)
                .map_err(|error| format!("invalid telemetry acknowledgement: {error}"))?;
            let expected_sample_end = telemetry_samples
                .last()
                .map(|sample| sample.sample_sequence)
                .ok_or_else(|| "telemetry batch is empty".to_string())?;
            if receipt.control_sequence_ack != *sequence
                || receipt.sample_sequence_end != expected_sample_end
                || receipt.batch_hash != batch_hash
            {
                return Err("telemetry acknowledgement does not match sent batch".to_string());
            }
            update_remote_session_sequence(*sequence)?;
            update_remote_telemetry_sequence(expected_sample_end)?;
            *telemetry_sequence = expected_sample_end;
            telemetry_samples.clear();
            Ok(TelemetryOutcome::Accepted(ProofTelemetryWindow {
                batch_hash,
                samples: submitted_samples,
            }))
        }
        "telemetry_rejected" => {
            let message = response.payload["message"]
                .as_str()
                .unwrap_or("control plane rejected telemetry batch")
                .to_string();
            *sequence = response.sequence_ack;
            update_remote_session_sequence(*sequence)?;
            telemetry_samples.clear();
            Ok(TelemetryOutcome::Rejected(message))
        }
        "session_revoked" => Ok(TelemetryOutcome::SessionRevoked),
        "error" => Err(response.payload["message"]
            .as_str()
            .unwrap_or("control plane rejected telemetry batch")
            .to_string()),
        other => Err(format!("unexpected telemetry control message {other}")),
    }
}

fn log_telemetry_unavailable(error: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "event": "gpu_telemetry_unavailable",
            "error": error,
        })
    );
}

fn log_telemetry_rejected(error: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "event": "gpu_telemetry_rejected",
            "error": error,
        })
    );
}

fn log_proof_worker_shutdown_error(failure_kind: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "event": "remote_proof_worker_shutdown_failed",
            "failure_kind": failure_kind,
        })
    );
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

fn ensure_credential_fresh(
    cancellation: &SessionOperationCancellation,
) -> Result<RemoteEnrollmentState, ReconnectFailure> {
    ensure_session_not_cancelled(cancellation)?;
    let state = load_remote_enrollment().map_err(|error| {
        ReconnectFailure::terminal(
            "local_state",
            format!("failed to load remote enrollment: {error}"),
        )
    })?;
    ensure_session_not_cancelled(cancellation)?;
    let refresh_due = credential_refresh_due(&state).map_err(|error| {
        ReconnectFailure::terminal(
            "local_state",
            format!("failed to evaluate credential expiry: {error}"),
        )
    })?;
    if refresh_due {
        ensure_session_not_cancelled(cancellation)?;
        refresh_credential_checked().map_err(classify_control_plane_error)?;
        ensure_session_not_cancelled(cancellation)?;
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
    use crate::exit_status::AgentExitCategory;

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
    #[tokio::test]
    async fn proof_worker_exit_is_propagated_by_the_supervisor() {
        let clean_worker = tokio::spawn(async { Ok(()) });
        assert!(finish_proof_worker(clean_worker, Ok(())).await.is_ok());

        let failed_worker = tokio::spawn(async { Err("state unavailable".to_string()) });
        let error = finish_proof_worker(failed_worker, Ok(()))
            .await
            .unwrap_err();
        assert_eq!(error.category(), AgentExitCategory::Internal);
        assert_eq!(error.failure_kind(), "proof_worker");
        assert_eq!(error.diagnostic_detail(), "state unavailable");

        let clean_exit = unexpected_proof_worker_exit(Ok(Ok(())));
        assert_eq!(clean_exit.category(), AgentExitCategory::Internal);
        assert_eq!(clean_exit.failure_kind(), "proof_worker");
        assert_eq!(
            clean_exit.diagnostic_detail(),
            "remote proof worker stopped unexpectedly"
        );
        let failed_exit = unexpected_proof_worker_exit(Ok(Err("state unavailable".to_string())));
        assert_eq!(failed_exit.category(), AgentExitCategory::Internal);
        assert_eq!(failed_exit.failure_kind(), "proof_worker");
        assert_eq!(
            failed_exit.diagnostic_detail(),
            "remote proof worker failed: state unavailable"
        );
    }

    static PREPARATION_EXECUTOR_STARTED: AtomicBool = AtomicBool::new(false);

    #[tokio::test]
    async fn shutdown_cancels_active_connection_preparation() {
        PREPARATION_EXECUTOR_STARTED.store(false, Ordering::SeqCst);
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let shutdown_task = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(1), async {
                while !PREPARATION_EXECUTOR_STARTED.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("connection preparation did not start");
            shutdown_tx
                .send(true)
                .expect("preparation shutdown receiver was dropped");
        });
        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            run_preparation_or_shutdown(
                "burd-agent-test".to_string(),
                cooperative_preparation_executor,
                &mut shutdown_rx,
            ),
        )
        .await
        .expect("connection preparation ignored shutdown")
        .expect("connection preparation returned an unexpected failure");
        assert!(outcome.is_none());
        shutdown_task.await.unwrap();
    }

    fn cooperative_preparation_executor(
        request: PreparationRequest,
    ) -> Result<PreparedConnection, ReconnectFailure> {
        PREPARATION_EXECUTOR_STARTED.store(true, Ordering::SeqCst);
        while request.cancellation.ensure_not_cancelled().is_ok() {
            std::thread::sleep(Duration::from_millis(1));
        }
        request.ensure_not_cancelled()?;
        unreachable!("cancelled preparation executor continued")
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
    fn websocket_statuses_preserve_terminal_failure_categories() {
        assert!(matches!(
            websocket_http_outcome(410),
            Some(ConnectionOutcome::RestartSession(_))
        ));
        for (status, expected_kind) in [
            (400, "control_plane_contract"),
            (401, "unauthorized"),
            (403, "session_revoked"),
        ] {
            let Some(ConnectionOutcome::Terminal { kind, .. }) = websocket_http_outcome(status)
            else {
                panic!("HTTP {status} did not produce a terminal outcome");
            };
            assert_eq!(kind, expected_kind, "HTTP {status}");
        }
        assert!(websocket_http_outcome(503).is_none());

        let missing_without_envelope = ControlPlaneRequestError::Rejected {
            status: 404,
            code: "remote_error".to_string(),
            message: "control plane rejected request".to_string(),
        };
        assert!(persisted_session_should_restart(&missing_without_envelope));
    }

    #[test]
    fn persisted_sessions_are_bound_to_the_enrollment_control_plane() {
        let session = RemoteSessionState {
            control_plane_url: "https://api.burd.cloud".to_string(),
            session_id: "session_1".to_string(),
            resume_token: "secret".to_string(),
            expires_at: "2026-08-01T00:00:00Z".to_string(),
            heartbeat_interval_seconds: 15,
            missed_heartbeat_limit: 3,
            sequence_last: 4,
            telemetry_sequence_last: 2,
            control_url: "wss://api.burd.cloud/v1/sessions/session_1/control".to_string(),
        };

        assert!(session_belongs_to_control_plane(
            &session,
            "https://api.burd.cloud/"
        ));
        assert!(!session_belongs_to_control_plane(
            &session,
            "https://staging-api.burd.cloud"
        ));
    }
}
