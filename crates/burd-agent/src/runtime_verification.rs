use crate::docker_runtime_backend::{
    DockerCommandControl, DockerContainerPlan, DockerRuntimeBackend, LinuxNativeDockerBackend,
    WindowsWsl2DockerBackend,
};
use crate::provider_job_executor::JobCancellation;
use crate::remote_enrollment::{ControlPlaneRequestError, join_url};
use burd_bench::build_registration_payload;
use burd_protocol::{
    AGENT_RUNTIME_CONTRACT_VERSION, NextRuntimeVerificationChallengeResponse,
    RUNTIME_PROOF_OUTPUT_SCHEMA_VERSION, RUNTIME_PROOF_POLICY_VERSION,
    RUNTIME_VERIFICATION_CANONICALIZATION_VERSION, RUNTIME_VERIFICATION_RESPONSE_SCHEMA_VERSION,
    RuntimeProofOutput, RuntimeVerificationChallenge, RuntimeVerificationEvidence,
    RuntimeVerificationResponsePayload, SignedRuntimeVerificationResponse,
    SubmitRuntimeVerificationResponse, fingerprint_claims, load_identity, load_private_key,
    load_remote_enrollment, load_remote_session, runtime_verification_fingerprint,
    runtime_verification_response_hash, runtime_verification_signature_message, sha256_hex,
    sign_message, validate_runtime_verification_challenge, validate_runtime_verification_evidence,
};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::sync::watch;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(20);
const MONITOR_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct RuntimeVerificationExecutionRequest {
    pub challenge: RuntimeVerificationChallenge,
    pub cancellation: JobCancellation,
}

pub type RuntimeVerificationExecutor =
    fn(RuntimeVerificationExecutionRequest) -> Result<RuntimeVerificationEvidence, String>;

pub async fn run_worker(
    agent_version: String,
    executor: RuntimeVerificationExecutor,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let challenge = match tokio::task::spawn_blocking(fetch_next_challenge).await {
            Ok(Ok(challenge)) => challenge,
            Ok(Err(ControlPlaneRequestError::Rejected { status: 404, .. })) => {
                wait_for_poll_or_shutdown(&mut shutdown).await;
                continue;
            }
            Ok(Err(error)) => {
                log_event(
                    "runtime_verification_poll_failed",
                    None,
                    Some(&error.to_string()),
                );
                wait_for_poll_or_shutdown(&mut shutdown).await;
                continue;
            }
            Err(error) => {
                log_event(
                    "runtime_verification_poll_task_failed",
                    None,
                    Some(&error.to_string()),
                );
                wait_for_poll_or_shutdown(&mut shutdown).await;
                continue;
            }
        };
        let fingerprint_version = agent_version.clone();
        let current_fingerprint = tokio::task::spawn_blocking(move || {
            build_registration_payload(&fingerprint_version).hardware_fingerprint
        })
        .await
        .map_err(|error| format!("runtime verification fingerprint task failed: {error}"))?;
        if let Err(error) = validate_challenge_context(&challenge, &current_fingerprint) {
            log_event(
                "runtime_verification_challenge_rejected",
                Some(&challenge.challenge_id),
                Some(&error),
            );
            wait_for_poll_or_shutdown(&mut shutdown).await;
            continue;
        }

        let started_at = Utc::now();
        let cancellation = JobCancellation::default();
        let request = RuntimeVerificationExecutionRequest {
            challenge: challenge.clone(),
            cancellation: cancellation.clone(),
        };
        let mut task = tokio::task::spawn_blocking(move || executor(request));
        let execution = tokio::select! {
            biased;
            _ = wait_for_shutdown(&mut shutdown) => {
                cancellation.cancel();
                let _ = tokio::time::timeout(CLEANUP_TIMEOUT, &mut task).await;
                return Ok(());
            }
            result = &mut task => result,
        };
        let evidence = match execution {
            Ok(Ok(evidence)) => evidence,
            Ok(Err(error)) => {
                log_event(
                    "runtime_verification_execution_failed",
                    Some(&challenge.challenge_id),
                    Some(&error),
                );
                wait_for_poll_or_shutdown(&mut shutdown).await;
                continue;
            }
            Err(error) => {
                log_event(
                    "runtime_verification_executor_task_failed",
                    Some(&challenge.challenge_id),
                    Some(&error.to_string()),
                );
                wait_for_poll_or_shutdown(&mut shutdown).await;
                continue;
            }
        };
        let completed_at = Utc::now();
        let signed = build_signed_response(&challenge, evidence, started_at, completed_at)?;
        let submit_challenge = challenge.clone();
        let submitted =
            tokio::task::spawn_blocking(move || submit_response(&submit_challenge, &signed))
                .await
                .map_err(|error| format!("runtime verification submission task failed: {error}"))?;
        match submitted {
            Ok(response) => log_event(
                "runtime_verification_submitted",
                Some(&response.challenge_id),
                Some(&response.status),
            ),
            Err(error) => log_event(
                "runtime_verification_submission_failed",
                Some(&challenge.challenge_id),
                Some(&error.to_string()),
            ),
        }
        wait_for_poll_or_shutdown(&mut shutdown).await;
    }
}

pub fn execute_runtime_verification(
    request: RuntimeVerificationExecutionRequest,
) -> Result<RuntimeVerificationEvidence, String> {
    validate_runtime_verification_challenge(&request.challenge)?;
    match request.challenge.runtime_backend.as_str() {
        "docker_linux_native" => execute_with_backend(
            &LinuxNativeDockerBackend::default(),
            &request.challenge,
            &request.cancellation,
        ),
        "docker_wsl2" => execute_with_backend(
            &WindowsWsl2DockerBackend::default(),
            &request.challenge,
            &request.cancellation,
        ),
        _ => Err("runtime verification backend is unsupported".to_string()),
    }
}

fn execute_with_backend<B: DockerRuntimeBackend>(
    backend: &B,
    challenge: &RuntimeVerificationChallenge,
    cancellation: &JobCancellation,
) -> Result<RuntimeVerificationEvidence, String> {
    let expires_at = parse_time(&challenge.expires_at)?;
    let timeout = (expires_at - Utc::now())
        .to_std()
        .map_err(|_| "runtime verification challenge is expired".to_string())?;
    if timeout < Duration::from_secs(10) {
        return Err("runtime verification challenge has insufficient time remaining".to_string());
    }
    let started = Instant::now();
    let plan = build_plan(challenge);
    let control = active_control(timeout, cancellation);
    backend
        .verify_environment(&plan, &control)
        .map_err(runtime_error)?;
    let environment = backend
        .runtime_environment(&control)
        .map_err(runtime_error)?;
    if !environment
        .nvidia_driver_version
        .chars()
        .all(|character| character.is_ascii_graphic())
    {
        return Err("runtime verification driver version is invalid".to_string());
    }

    if let Some(existing) = backend
        .existing_container(&plan.name, &control)
        .map_err(runtime_error)?
    {
        if existing
            .labels
            .get("com.burd.runtime_verification")
            .map(String::as_str)
            != Some("true")
            || existing
                .labels
                .get("com.burd.challenge_id")
                .map(String::as_str)
                != Some(challenge.challenge_id.as_str())
        {
            return Err(
                "runtime verification container name is owned by another execution".to_string(),
            );
        }
        backend
            .remove(&plan.name, &DockerCommandControl::cleanup(CLEANUP_TIMEOUT))
            .map_err(runtime_error)?;
    }

    let container_id = backend.create(&plan, &control).map_err(runtime_error)?;
    if let Err(error) = backend.start(&container_id, &control) {
        let _ = backend.remove(
            &container_id,
            &DockerCommandControl::cleanup(CLEANUP_TIMEOUT),
        );
        return Err(runtime_error(error));
    }

    let state = loop {
        if cancellation.requested() {
            cleanup_running(backend, &container_id);
            return Err("runtime verification execution was cancelled".to_string());
        }
        if Utc::now() >= expires_at || started.elapsed() >= timeout {
            cleanup_running(backend, &container_id);
            return Err("runtime verification execution timed out".to_string());
        }
        let state = backend.inspect(&container_id, &control).map_err(|error| {
            cleanup_running(backend, &container_id);
            runtime_error(error)
        })?;
        if !state.running {
            break state;
        }
        std::thread::sleep(MONITOR_INTERVAL);
    };
    let logs = backend.logs(&container_id, &control).map_err(|error| {
        cleanup_running(backend, &container_id);
        runtime_error(error)
    })?;
    backend
        .remove(
            &container_id,
            &DockerCommandControl::cleanup(CLEANUP_TIMEOUT),
        )
        .map_err(runtime_error)?;
    if state.oom_killed || state.exit_code != Some(0) {
        return Err("runtime verification container failed".to_string());
    }
    if logs.stdout_truncated() || logs.stderr_truncated() {
        return Err("runtime verification output was truncated".to_string());
    }
    let proof: RuntimeProofOutput = serde_json::from_str(logs.stdout_tail().trim())
        .map_err(|_| "runtime verification output is invalid".to_string())?;
    if proof.schema_version != RUNTIME_PROOF_OUTPUT_SCHEMA_VERSION
        || proof.nonce != challenge.nonce
        || proof.observed_gpu_uuids != [challenge.gpu_uuid.clone()]
        || proof.nvidia_driver_version != environment.nvidia_driver_version
    {
        return Err("runtime verification proof output does not match the challenge".to_string());
    }
    let evidence = RuntimeVerificationEvidence {
        host_os: challenge.host_os.clone(),
        runtime_backend: backend.runtime_backend().to_string(),
        container_os: "linux".to_string(),
        gpu_backend: "cuda".to_string(),
        gpu_runtime: "nvidia".to_string(),
        isolation_mode: "linux_container".to_string(),
        docker_server_version: environment.docker_server_version,
        nvidia_driver_version: proof.nvidia_driver_version,
        nvidia_runtime: environment.nvidia_runtime,
        cuda_runtime_version: proof.cuda_runtime_version,
        observed_gpu_uuids: proof.observed_gpu_uuids,
        proof_image_digest: challenge.proof_image_ref.clone(),
        proof_nonce: proof.nonce,
        network_mode: "none".to_string(),
        run_as_user: "1000:1000".to_string(),
        read_only_rootfs: true,
        no_new_privileges: true,
        cap_drop: vec!["ALL".to_string()],
    };
    validate_runtime_verification_evidence(challenge, &evidence)?;
    Ok(evidence)
}

fn build_plan(challenge: &RuntimeVerificationChallenge) -> DockerContainerPlan {
    let suffix = sha256_hex(challenge.challenge_id.as_bytes());
    let mut labels = BTreeMap::new();
    labels.insert("com.burd.managed".to_string(), "true".to_string());
    labels.insert(
        "com.burd.runtime_verification".to_string(),
        "true".to_string(),
    );
    labels.insert(
        "com.burd.challenge_id".to_string(),
        challenge.challenge_id.clone(),
    );
    labels.insert(
        "com.burd.provider_id".to_string(),
        challenge.provider_id.clone(),
    );
    labels.insert(
        "com.burd.device_id".to_string(),
        challenge.device_id.clone(),
    );
    labels.insert(
        "com.burd.session_id".to_string(),
        challenge.session_id.clone(),
    );
    labels.insert("com.burd.gpu_uuid".to_string(), challenge.gpu_uuid.clone());
    let environment = BTreeMap::from([
        (
            "BURD_RUNTIME_PROOF_NONCE".to_string(),
            challenge.nonce.clone(),
        ),
        (
            "BURD_RUNTIME_PROOF_SCHEMA".to_string(),
            RUNTIME_PROOF_OUTPUT_SCHEMA_VERSION.to_string(),
        ),
    ]);
    DockerContainerPlan {
        name: format!("burd-runtime-proof-{}", &suffix[..16]),
        image_ref: challenge.proof_image_ref.clone(),
        gpu_uuid: challenge.gpu_uuid.clone(),
        user: "1000:1000".to_string(),
        cpu_millis: 1_000,
        memory_mib: 1_024,
        pids_limit: 64,
        shm_size_mib: 64,
        labels,
        environment,
        artifact_workspace: false,
        input_artifact_count: 0,
        output_artifact_count: 0,
        input_artifact_bytes: 0,
        output_artifact_bytes: 0,
    }
}

fn cleanup_running<B: DockerRuntimeBackend>(backend: &B, container_id: &str) {
    let _ = backend.terminate(
        container_id,
        &DockerCommandControl::cleanup(CLEANUP_TIMEOUT),
    );
    let _ = backend.kill(
        container_id,
        &DockerCommandControl::cleanup(CLEANUP_TIMEOUT),
    );
    let _ = backend.remove(
        container_id,
        &DockerCommandControl::cleanup(CLEANUP_TIMEOUT),
    );
}

fn active_control(timeout: Duration, cancellation: &JobCancellation) -> DockerCommandControl {
    DockerCommandControl::cancellable(timeout.min(COMMAND_TIMEOUT), cancellation.clone())
}

fn runtime_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn validate_challenge_context(
    challenge: &RuntimeVerificationChallenge,
    current_fingerprint: &str,
) -> Result<(), String> {
    validate_runtime_verification_challenge(challenge)?;
    let enrollment = load_remote_enrollment()?;
    let session = load_remote_session()?;
    if enrollment.provider_id != challenge.provider_id
        || enrollment.device_id != challenge.device_id
        || session.session_id != challenge.session_id
        || current_fingerprint != challenge.hardware_fingerprint
        || challenge.proof_policy_version != RUNTIME_PROOF_POLICY_VERSION
        || challenge.agent_runtime_contract_version != AGENT_RUNTIME_CONTRACT_VERSION
    {
        return Err("runtime verification challenge does not match local identity".to_string());
    }
    if parse_time(&challenge.expires_at)? <= Utc::now() {
        return Err("runtime verification challenge is expired".to_string());
    }
    Ok(())
}

fn build_signed_response(
    challenge: &RuntimeVerificationChallenge,
    evidence: RuntimeVerificationEvidence,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
) -> Result<SignedRuntimeVerificationResponse, String> {
    validate_runtime_verification_evidence(challenge, &evidence)?;
    if completed_at >= parse_time(&challenge.expires_at)? {
        return Err("runtime verification completed after challenge expiration".to_string());
    }
    let enrollment = load_remote_enrollment()?;
    let identity = load_identity()?;
    let private_key = load_private_key(&identity)?;
    let fingerprint = runtime_verification_fingerprint(&fingerprint_claims(challenge, &evidence))?;
    let payload = RuntimeVerificationResponsePayload {
        schema_version: RUNTIME_VERIFICATION_RESPONSE_SCHEMA_VERSION.to_string(),
        challenge_id: challenge.challenge_id.clone(),
        nonce: challenge.nonce.clone(),
        provider_id: challenge.provider_id.clone(),
        device_id: challenge.device_id.clone(),
        session_id: challenge.session_id.clone(),
        hardware_fingerprint: challenge.hardware_fingerprint.clone(),
        gpu_uuid: challenge.gpu_uuid.clone(),
        runtime_backend: challenge.runtime_backend.clone(),
        proof_policy_version: challenge.proof_policy_version.clone(),
        agent_runtime_contract_version: challenge.agent_runtime_contract_version.clone(),
        runtime_verification_fingerprint: fingerprint,
        evidence,
        started_at: started_at.to_rfc3339(),
        completed_at: completed_at.to_rfc3339(),
    };
    let response_hash = runtime_verification_response_hash(&payload)?;
    let message = runtime_verification_signature_message(
        &payload,
        &response_hash,
        &enrollment.public_key_id,
    )?;
    let signature = sign_message(&private_key.secret_key_base64, message.as_bytes())?;
    Ok(SignedRuntimeVerificationResponse {
        payload,
        response_hash,
        public_key_id: enrollment.public_key_id,
        signature,
        canonicalization_version: RUNTIME_VERIFICATION_CANONICALIZATION_VERSION.to_string(),
    })
}

fn fetch_next_challenge() -> Result<RuntimeVerificationChallenge, ControlPlaneRequestError> {
    let enrollment = load_remote_enrollment().map_err(ControlPlaneRequestError::LocalState)?;
    let session = load_remote_session().map_err(ControlPlaneRequestError::LocalState)?;
    let url = join_url(
        &session.control_plane_url,
        &format!(
            "/v1/sessions/{}/runtime-verifications/next",
            session.session_id
        ),
    );
    let mut response = ureq::get(&url)
        .header(
            "Authorization",
            &format!("Bearer {}", enrollment.credential),
        )
        .header("X-Burd-Session-Token", &session.resume_token)
        .header("X-Burd-Device-Id", &enrollment.device_id)
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|error| ControlPlaneRequestError::Transport(error.to_string()))?;
    let status = response.status();
    let value = response.body_mut().read_json::<serde_json::Value>();
    if !status.is_success() {
        return Err(response_error(status.as_u16(), value.ok()));
    }
    let response: NextRuntimeVerificationChallengeResponse = serde_json::from_value(
        value.map_err(|error| ControlPlaneRequestError::Contract(error.to_string()))?,
    )
    .map_err(|error| ControlPlaneRequestError::Contract(error.to_string()))?;
    Ok(response.challenge)
}

fn submit_response(
    challenge: &RuntimeVerificationChallenge,
    signed: &SignedRuntimeVerificationResponse,
) -> Result<SubmitRuntimeVerificationResponse, ControlPlaneRequestError> {
    let enrollment = load_remote_enrollment().map_err(ControlPlaneRequestError::LocalState)?;
    let session = load_remote_session().map_err(ControlPlaneRequestError::LocalState)?;
    if session.session_id != challenge.session_id {
        return Err(ControlPlaneRequestError::LocalState(
            "remote session changed before runtime verification submission".to_string(),
        ));
    }
    let url = join_url(
        &session.control_plane_url,
        &format!(
            "/v1/sessions/{}/runtime-verifications/{}/response",
            session.session_id, challenge.challenge_id
        ),
    );
    let mut response = ureq::post(&url)
        .header(
            "Authorization",
            &format!("Bearer {}", enrollment.credential),
        )
        .header("X-Burd-Session-Token", &session.resume_token)
        .header("X-Burd-Device-Id", &enrollment.device_id)
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .send_json(signed)
        .map_err(|error| ControlPlaneRequestError::Transport(error.to_string()))?;
    let status = response.status();
    let value = response.body_mut().read_json::<serde_json::Value>();
    if !status.is_success() {
        return Err(response_error(status.as_u16(), value.ok()));
    }
    serde_json::from_value(
        value.map_err(|error| ControlPlaneRequestError::Contract(error.to_string()))?,
    )
    .map_err(|error| ControlPlaneRequestError::Contract(error.to_string()))
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
            .unwrap_or("control plane rejected runtime verification request")
            .to_string(),
    }
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| "runtime verification timestamp is invalid".to_string())
}

async fn wait_for_poll_or_shutdown(shutdown: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::time::sleep(POLL_INTERVAL) => {}
        _ = shutdown.changed() => {}
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow() {
            return;
        }
    }
}

fn log_event(event: &str, challenge_id: Option<&str>, detail: Option<&str>) {
    println!(
        "{}",
        serde_json::json!({
            "event": event,
            "challenge_id": challenge_id,
            "detail": detail.map(|value| value.chars().take(160).collect::<String>()),
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{
        RUNTIME_PROOF_POLICY_VERSION, RUNTIME_VERIFICATION_CHALLENGE_SCHEMA_VERSION,
    };

    fn challenge() -> RuntimeVerificationChallenge {
        RuntimeVerificationChallenge {
            schema_version: RUNTIME_VERIFICATION_CHALLENGE_SCHEMA_VERSION.to_string(),
            challenge_id: "runtime_challenge_test".to_string(),
            nonce: "burd_runtime_nonce_test".to_string(),
            provider_id: "provider_test".to_string(),
            device_id: "device_test".to_string(),
            session_id: "session_test".to_string(),
            hardware_fingerprint: "a".repeat(64),
            host_os: "linux".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            runtime_backend: "docker_linux_native".to_string(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            proof_image_ref: format!("ghcr.io/burd/runtime-proof@sha256:{}", "b".repeat(64)),
            proof_policy_version: RUNTIME_PROOF_POLICY_VERSION.to_string(),
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            issued_at: "2026-08-08T00:00:00Z".to_string(),
            expires_at: "2026-08-08T00:10:00Z".to_string(),
            verification_ttl_seconds: 86_400,
        }
    }

    #[test]
    fn proof_plan_is_offline_non_root_and_nonce_bound() {
        let challenge = challenge();
        let plan = build_plan(&challenge);
        let args = LinuxNativeDockerBackend::create_args(&plan);
        assert!(args.windows(2).any(|pair| pair == ["--network", "none"]));
        assert!(args.windows(2).any(|pair| pair == ["--user", "1000:1000"]));
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--env" && pair[1] == "BURD_RUNTIME_PROOF_NONCE=burd_runtime_nonce_test"
        }));
        assert!(args.iter().any(|value| value == "--read-only"));
        assert!(!args.iter().any(|value| value == "--privileged"));
        assert!(!args.iter().any(|value| value == "--mount"));
    }

    #[test]
    fn proof_container_name_does_not_expose_identifiers() {
        let challenge = challenge();
        let plan = build_plan(&challenge);
        assert!(plan.name.starts_with("burd-runtime-proof-"));
        assert!(!plan.name.contains(&challenge.challenge_id));
        assert!(!plan.name.contains(&challenge.nonce));
    }

    #[test]
    #[ignore = "requires a multi-GPU NVIDIA host, Docker, and a local digest-pinned runtime-proof image implementing the proof output contract"]
    fn physical_runtime_proof_requires_multi_gpu_and_exact_binding() {
        let image_ref = std::env::var("BURD_RUNTIME_PROOF_IMAGE_REF")
            .expect("BURD_RUNTIME_PROOF_IMAGE_REF is required");
        let gpu_uuid = std::env::var("BURD_RUNTIME_PROOF_GPU_UUID")
            .expect("BURD_RUNTIME_PROOF_GPU_UUID is required");
        let output = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=uuid", "--format=csv,noheader"])
            .output()
            .expect("nvidia-smi must be executable");
        assert!(output.status.success(), "nvidia-smi GPU query failed");
        let stdout = String::from_utf8(output.stdout).expect("nvidia-smi output must be UTF-8");
        let host_gpu_uuids = stdout
            .lines()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        assert!(
            host_gpu_uuids.len() >= 2,
            "physical isolation gate requires at least two host GPUs"
        );
        assert!(
            host_gpu_uuids
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&gpu_uuid)),
            "challenged GPU must exist on the host"
        );
        assert!(
            host_gpu_uuids
                .iter()
                .any(|value| !value.eq_ignore_ascii_case(&gpu_uuid)),
            "physical isolation gate requires another host GPU"
        );

        let now = Utc::now();
        let mut challenge = challenge();
        challenge.challenge_id = "runtime_challenge_physical_gate".to_string();
        challenge.nonce = "burd_runtime_physical_gate_nonce".to_string();
        challenge.host_os = std::env::consts::OS.to_string();
        challenge.runtime_backend = match std::env::consts::OS {
            "linux" => "docker_linux_native".to_string(),
            "windows" => "docker_wsl2".to_string(),
            other => panic!("unsupported physical gate host OS: {other}"),
        };
        challenge.gpu_uuid = gpu_uuid.clone();
        challenge.proof_image_ref = image_ref;
        challenge.issued_at = (now - chrono::Duration::seconds(1)).to_rfc3339();
        challenge.expires_at = (now + chrono::Duration::minutes(10)).to_rfc3339();

        let evidence = execute_runtime_verification(RuntimeVerificationExecutionRequest {
            challenge,
            cancellation: JobCancellation::default(),
        })
        .expect("runtime proof must complete");
        assert!(
            evidence.observed_gpu_uuids.len() == 1
                && evidence.observed_gpu_uuids[0].eq_ignore_ascii_case(&gpu_uuid),
            "runtime proof must bind only the selected physical GPU"
        );
    }
}
