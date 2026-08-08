use crate::docker_runtime_backend::{
    DockerCommandControl, DockerRuntimeBackend, DockerRuntimeEnvironment, LinuxNativeDockerBackend,
    WindowsWsl2DockerBackend,
};
use crate::provider_job_executor::JobCancellation;
use crate::remote_enrollment::{ControlPlaneRequestError, join_url};
use burd_bench::build_registration_payload;
use burd_protocol::{
    AGENT_RUNTIME_CONTRACT_VERSION, PROVIDER_RUNTIME_OBSERVATION_SCHEMA_VERSION,
    ProviderRuntimeObservationPayload, RUNTIME_VERIFICATION_CANONICALIZATION_VERSION,
    SignedProviderRuntimeObservation, SubmitProviderRuntimeObservationResponse, load_identity,
    load_private_key, load_remote_enrollment, load_remote_session,
    provider_runtime_observation_hash, provider_runtime_observation_signature_message,
    sign_message, validate_provider_runtime_observation_payload,
};
use chrono::Utc;
use std::time::Duration;
use tokio::sync::watch;

const OBSERVATION_INTERVAL: Duration = Duration::from_secs(60);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn run_worker(
    agent_version: String,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let cancellation = JobCancellation::default();
        let task_version = agent_version.clone();
        let task_cancellation = cancellation.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            let signed = build_signed_runtime_observation(&task_version, &task_cancellation)
                .map_err(ControlPlaneRequestError::LocalState)?;
            submit_runtime_observation(&signed)
        });
        let result = tokio::select! {
            biased;
            _ = wait_for_shutdown(&mut shutdown) => {
                cancellation.cancel();
                let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut task).await;
                return Ok(());
            }
            result = &mut task => result,
        };
        match result {
            Ok(Ok(response)) => log_event(
                "runtime_observation_submitted",
                Some(&response.observation_hash),
            ),
            Ok(Err(error)) => log_event("runtime_observation_failed", Some(&error.to_string())),
            Err(error) => log_event("runtime_observation_task_failed", Some(&error.to_string())),
        }
        wait_for_interval_or_shutdown(&mut shutdown).await;
    }
}

pub fn build_signed_runtime_observation(
    agent_version: &str,
    cancellation: &JobCancellation,
) -> Result<SignedProviderRuntimeObservation, String> {
    let enrollment = load_remote_enrollment()?;
    let session = load_remote_session()?;
    let identity = load_identity()?;
    let private_key = load_private_key(&identity)?;
    let hardware_fingerprint = build_registration_payload(agent_version).hardware_fingerprint;
    let (runtime_backend, environment) = match std::env::consts::OS {
        "linux" => observe_with_backend(&LinuxNativeDockerBackend::default(), cancellation)?,
        "windows" => observe_with_backend(&WindowsWsl2DockerBackend::default(), cancellation)?,
        _ => return Err("runtime observation host OS is unsupported".to_string()),
    };
    let payload = build_payload(
        &enrollment.provider_id,
        &enrollment.device_id,
        &session.session_id,
        hardware_fingerprint,
        std::env::consts::OS,
        runtime_backend,
        environment,
    );
    validate_provider_runtime_observation_payload(&payload)?;
    let observation_hash = provider_runtime_observation_hash(&payload)?;
    let message = provider_runtime_observation_signature_message(
        &payload,
        &observation_hash,
        &enrollment.public_key_id,
    )?;
    let signature = sign_message(&private_key.secret_key_base64, message.as_bytes())?;
    Ok(SignedProviderRuntimeObservation {
        payload,
        observation_hash,
        public_key_id: enrollment.public_key_id,
        signature,
        canonicalization_version: RUNTIME_VERIFICATION_CANONICALIZATION_VERSION.to_string(),
    })
}

fn observe_with_backend<B: DockerRuntimeBackend>(
    backend: &B,
    cancellation: &JobCancellation,
) -> Result<(&'static str, DockerRuntimeEnvironment), String> {
    let control = DockerCommandControl::cancellable(COMMAND_TIMEOUT, cancellation.clone());
    backend
        .verify_platform(&control)
        .map_err(|error| error.to_string())?;
    let environment = backend
        .runtime_environment(&control)
        .map_err(|error| error.to_string())?;
    Ok((backend.runtime_backend(), environment))
}

fn build_payload(
    provider_id: &str,
    device_id: &str,
    session_id: &str,
    hardware_fingerprint: String,
    host_os: &str,
    runtime_backend: &str,
    mut environment: DockerRuntimeEnvironment,
) -> ProviderRuntimeObservationPayload {
    environment
        .gpu_uuids
        .sort_by_key(|value| value.to_ascii_lowercase());
    ProviderRuntimeObservationPayload {
        schema_version: PROVIDER_RUNTIME_OBSERVATION_SCHEMA_VERSION.to_string(),
        provider_id: provider_id.to_string(),
        device_id: device_id.to_string(),
        session_id: session_id.to_string(),
        hardware_fingerprint,
        host_os: host_os.to_string(),
        runtime_backend: runtime_backend.to_string(),
        container_os: "linux".to_string(),
        gpu_backend: "cuda".to_string(),
        gpu_runtime: "nvidia".to_string(),
        isolation_mode: "linux_container".to_string(),
        docker_server_version: environment.docker_server_version,
        nvidia_driver_version: environment.nvidia_driver_version,
        nvidia_runtime: environment.nvidia_runtime,
        gpu_uuids: environment.gpu_uuids,
        agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
        observed_at: Utc::now().to_rfc3339(),
    }
}

fn submit_runtime_observation(
    signed: &SignedProviderRuntimeObservation,
) -> Result<SubmitProviderRuntimeObservationResponse, ControlPlaneRequestError> {
    let enrollment = load_remote_enrollment().map_err(ControlPlaneRequestError::LocalState)?;
    let session = load_remote_session().map_err(ControlPlaneRequestError::LocalState)?;
    if signed.payload.provider_id != enrollment.provider_id
        || signed.payload.device_id != enrollment.device_id
        || signed.payload.session_id != session.session_id
        || signed.public_key_id != enrollment.public_key_id
    {
        return Err(ControlPlaneRequestError::LocalState(
            "remote identity changed before runtime observation submission".to_string(),
        ));
    }
    let url = join_url(
        &session.control_plane_url,
        &format!("/v1/sessions/{}/runtime-observations", session.session_id),
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
        let value = value.unwrap_or(serde_json::Value::Null);
        return Err(ControlPlaneRequestError::Rejected {
            status: status.as_u16(),
            code: value["error"]["code"]
                .as_str()
                .unwrap_or("remote_error")
                .to_string(),
            message: value["error"]["message"]
                .as_str()
                .unwrap_or("control plane rejected runtime observation")
                .to_string(),
        });
    }
    serde_json::from_value(
        value.map_err(|error| ControlPlaneRequestError::Contract(error.to_string()))?,
    )
    .map_err(|error| ControlPlaneRequestError::Contract(error.to_string()))
}

async fn wait_for_interval_or_shutdown(shutdown: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::time::sleep(OBSERVATION_INTERVAL) => {}
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

fn log_event(event: &str, detail: Option<&str>) {
    println!(
        "{}",
        serde_json::json!({
            "event": event,
            "detail": detail.map(|value| value.chars().take(160).collect::<String>()),
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_payload_is_current_sorted_and_contains_no_credentials() {
        let payload = build_payload(
            "provider_1",
            "device_1",
            "session_2",
            "a".repeat(64),
            "linux",
            "docker_linux_native",
            DockerRuntimeEnvironment {
                docker_server_version: "28.3.0".to_string(),
                nvidia_driver_version: "580.1".to_string(),
                nvidia_runtime: "nvidia".to_string(),
                gpu_uuids: vec!["GPU-B".to_string(), "GPU-A".to_string()],
            },
        );
        validate_provider_runtime_observation_payload(&payload).unwrap();
        assert_eq!(payload.gpu_uuids, ["GPU-A", "GPU-B"]);
        let serialized = serde_json::to_string(&payload).unwrap();
        for forbidden in ["credential", "resume_token", "private_key", "bearer"] {
            assert!(!serialized.to_ascii_lowercase().contains(forbidden));
        }
    }
}
