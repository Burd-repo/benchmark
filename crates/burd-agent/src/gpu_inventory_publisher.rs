use crate::remote_enrollment::{ControlPlaneRequestError, join_url};
use burd_bench::build_registration_payload;
use burd_hardware::NvidiaTelemetryCollection;
use burd_protocol::{
    DEVICE_GPU_INVENTORY_CANONICALIZATION_VERSION, DEVICE_GPU_INVENTORY_SCHEMA_VERSION,
    DeviceGpuInventoryGpu, DeviceGpuInventoryPayload, SignedDeviceGpuInventory,
    SubmitDeviceGpuInventoryResponse, device_gpu_inventory_hash,
    device_gpu_inventory_signature_message, hash_canonical, load_identity, load_private_key,
    load_remote_enrollment, load_remote_session, sign_message,
    validate_device_gpu_inventory_payload,
};
use chrono::Utc;
use serde::Serialize;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;

const INVENTORY_PROBE_INTERVAL: Duration = Duration::from_secs(60);
const STATE_POLL_INTERVAL: Duration = Duration::from_secs(2);
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);

pub(crate) type GpuInventoryCollector = fn(u64) -> Result<NvidiaTelemetryCollection, String>;

#[derive(Debug)]
enum GpuInventoryDiscovery {
    Present(NvidiaTelemetryCollection),
    Empty(NvidiaTelemetryCollection),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationBinding {
    control_plane_url: String,
    provider_id: String,
    device_id: String,
    session_id: String,
    public_key_id: String,
}

struct SigningContext {
    binding: PublicationBinding,
    hardware_fingerprint: String,
    secret_key_base64: String,
}

#[derive(Clone)]
struct PreparedInventory {
    signed: SignedDeviceGpuInventory,
    publication_fingerprint: String,
    binding: PublicationBinding,
}

#[derive(Serialize)]
struct PublicationFingerprintClaims<'a> {
    domain: &'static str,
    provider_id: &'a str,
    device_id: &'a str,
    session_id: &'a str,
    hardware_fingerprint: &'a str,
    public_key_id: &'a str,
    gpus: &'a [DeviceGpuInventoryGpu],
}

#[derive(Default)]
struct RetryBackoff {
    failures: u32,
}

impl RetryBackoff {
    fn next_delay(&mut self) -> Duration {
        let multiplier = 1_u64 << self.failures.min(5);
        self.failures = self.failures.saturating_add(1);
        RETRY_MIN.saturating_mul(multiplier as u32).min(RETRY_MAX)
    }

    fn reset(&mut self) {
        self.failures = 0;
    }
}

enum RetryWait {
    Elapsed,
    BindingChanged,
    Shutdown,
}

pub async fn run_worker(
    agent_version: String,
    collector: GpuInventoryCollector,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut last_published: Option<(PublicationBinding, String)> = None;
    let mut pending: Option<PreparedInventory> = None;
    let mut next_probe = Instant::now();
    let mut backoff = RetryBackoff::default();

    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let binding = match tokio::task::spawn_blocking(load_publication_binding).await {
            Ok(Ok(binding)) => binding,
            Ok(Err(_)) => {
                log_event(
                    "gpu_inventory_waiting_for_session",
                    Some("local_state_unavailable"),
                    None,
                );
                if wait_for_duration_or_shutdown(STATE_POLL_INTERVAL, &mut shutdown).await {
                    return Ok(());
                }
                continue;
            }
            Err(_) => {
                log_event("gpu_inventory_task_failed", Some("join_error"), None);
                if wait_for_duration_or_shutdown(STATE_POLL_INTERVAL, &mut shutdown).await {
                    return Ok(());
                }
                continue;
            }
        };

        if pending
            .as_ref()
            .is_some_and(|prepared| prepared.binding != binding)
        {
            pending = None;
            backoff.reset();
            next_probe = Instant::now();
        }

        let binding_changed = last_published
            .as_ref()
            .is_none_or(|(published, _)| published != &binding);
        if pending.is_none() && (binding_changed || Instant::now() >= next_probe) {
            let task_version = agent_version.clone();
            let mut task = tokio::task::spawn_blocking(move || {
                build_signed_gpu_inventory(&task_version, collector)
            });
            let result = tokio::select! {
                biased;
                _ = wait_for_shutdown(&mut shutdown) => {
                    let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut task).await;
                    return Ok(());
                }
                result = &mut task => result,
            };
            let prepared = match result {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(_)) => {
                    log_event(
                        "gpu_inventory_discovery_failed",
                        Some("discovery_failed"),
                        None,
                    );
                    let delay = backoff.next_delay();
                    match wait_for_retry_or_binding_change(&binding, delay, &mut shutdown).await {
                        RetryWait::Shutdown => return Ok(()),
                        RetryWait::BindingChanged => backoff.reset(),
                        RetryWait::Elapsed => {}
                    }
                    continue;
                }
                Err(_) => {
                    log_event("gpu_inventory_task_failed", Some("join_error"), None);
                    let delay = backoff.next_delay();
                    if wait_for_duration_or_shutdown(delay, &mut shutdown).await {
                        return Ok(());
                    }
                    continue;
                }
            };
            if already_published(last_published.as_ref(), &prepared) {
                next_probe = Instant::now() + INVENTORY_PROBE_INTERVAL;
                backoff.reset();
            } else {
                pending = Some(prepared);
            }
        }

        let Some(prepared) = pending.clone() else {
            if wait_for_duration_or_shutdown(STATE_POLL_INTERVAL, &mut shutdown).await {
                return Ok(());
            }
            continue;
        };
        let inventory_hash = prepared.signed.inventory_hash.clone();
        let prepared_for_task = prepared.clone();
        let mut task =
            tokio::task::spawn_blocking(move || submit_gpu_inventory(&prepared_for_task));
        let result = tokio::select! {
            biased;
            _ = wait_for_shutdown(&mut shutdown) => {
                let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut task).await;
                return Ok(());
            }
            result = &mut task => result,
        };
        match result {
            Ok(Ok(_)) => {
                log_event("gpu_inventory_submitted", None, Some(&inventory_hash));
                last_published = Some((
                    prepared.binding.clone(),
                    prepared.publication_fingerprint.clone(),
                ));
                pending = None;
                next_probe = Instant::now() + INVENTORY_PROBE_INTERVAL;
                backoff.reset();
            }
            Ok(Err(error)) => {
                log_event(
                    "gpu_inventory_submission_failed",
                    Some(request_failure_reason(&error)),
                    None,
                );
                let delay = backoff.next_delay();
                match wait_for_retry_or_binding_change(&binding, delay, &mut shutdown).await {
                    RetryWait::Shutdown => return Ok(()),
                    RetryWait::BindingChanged => {
                        pending = None;
                        backoff.reset();
                        next_probe = Instant::now();
                    }
                    RetryWait::Elapsed => {}
                }
            }
            Err(_) => {
                log_event("gpu_inventory_task_failed", Some("join_error"), None);
                let delay = backoff.next_delay();
                if wait_for_duration_or_shutdown(delay, &mut shutdown).await {
                    return Ok(());
                }
            }
        }
    }
}

fn build_signed_gpu_inventory(
    agent_version: &str,
    collector: GpuInventoryCollector,
) -> Result<PreparedInventory, String> {
    let context = load_signing_context(agent_version)?;
    let collection = match discover_gpu_inventory(collector, 0) {
        GpuInventoryDiscovery::Present(collection) | GpuInventoryDiscovery::Empty(collection) => {
            collection
        }
        GpuInventoryDiscovery::Unavailable(error) => return Err(error),
    };
    prepare_inventory(context, collection, Utc::now().to_rfc3339())
}

fn discover_gpu_inventory(
    collector: GpuInventoryCollector,
    first_sample_sequence: u64,
) -> GpuInventoryDiscovery {
    match collector(first_sample_sequence) {
        Ok(collection) if collection.inventory.is_empty() => {
            GpuInventoryDiscovery::Empty(collection)
        }
        Ok(collection) => GpuInventoryDiscovery::Present(collection),
        Err(error) => GpuInventoryDiscovery::Unavailable(error),
    }
}

fn load_publication_binding() -> Result<PublicationBinding, String> {
    let enrollment = load_remote_enrollment()?;
    let session = load_remote_session()?;
    if enrollment.control_plane_url != session.control_plane_url {
        return Err("remote enrollment and session Control Plane URLs differ".to_string());
    }
    Ok(PublicationBinding {
        control_plane_url: session.control_plane_url,
        provider_id: enrollment.provider_id,
        device_id: enrollment.device_id,
        session_id: session.session_id,
        public_key_id: enrollment.public_key_id,
    })
}

fn load_signing_context(agent_version: &str) -> Result<SigningContext, String> {
    let binding = load_publication_binding()?;
    let identity = load_identity()?;
    let private_key = load_private_key(&identity)?;
    let hardware_fingerprint = build_registration_payload(agent_version).hardware_fingerprint;
    Ok(SigningContext {
        binding,
        hardware_fingerprint,
        secret_key_base64: private_key.secret_key_base64,
    })
}

fn prepare_inventory(
    context: SigningContext,
    collection: NvidiaTelemetryCollection,
    observed_at: String,
) -> Result<PreparedInventory, String> {
    let mut gpus = collection
        .inventory
        .into_iter()
        .map(|gpu| {
            Ok(DeviceGpuInventoryGpu {
                gpu_uuid: gpu.gpu_uuid,
                gpu_index: gpu.gpu_index,
                backend: "cuda".to_string(),
                pci_vendor_id: gpu
                    .pci_vendor_id
                    .ok_or_else(|| "GPU PCI vendor ID is unavailable".to_string())?,
                pci_device_id: gpu
                    .pci_device_id
                    .ok_or_else(|| "GPU PCI device ID is unavailable".to_string())?,
                vram_total_mib: Some(gpu.vram_total_mib),
                status: "active".to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    gpus.sort_by(|left, right| {
        (left.gpu_index, left.gpu_uuid.to_ascii_lowercase())
            .cmp(&(right.gpu_index, right.gpu_uuid.to_ascii_lowercase()))
    });
    let payload = DeviceGpuInventoryPayload {
        schema_version: DEVICE_GPU_INVENTORY_SCHEMA_VERSION.to_string(),
        provider_id: context.binding.provider_id.clone(),
        device_id: context.binding.device_id.clone(),
        session_id: context.binding.session_id.clone(),
        hardware_fingerprint: context.hardware_fingerprint,
        observed_at,
        gpus,
    };
    validate_device_gpu_inventory_payload(&payload)?;
    let inventory_hash = device_gpu_inventory_hash(&payload)?;
    let message = device_gpu_inventory_signature_message(
        &payload,
        &inventory_hash,
        &context.binding.public_key_id,
    )?;
    let signature = sign_message(&context.secret_key_base64, message.as_bytes())?;
    let publication_fingerprint = hash_canonical(&PublicationFingerprintClaims {
        domain: "burd.device-gpu-inventory-publication.v1",
        provider_id: &payload.provider_id,
        device_id: &payload.device_id,
        session_id: &payload.session_id,
        hardware_fingerprint: &payload.hardware_fingerprint,
        public_key_id: &context.binding.public_key_id,
        gpus: &payload.gpus,
    })?;
    Ok(PreparedInventory {
        signed: SignedDeviceGpuInventory {
            payload,
            inventory_hash,
            public_key_id: context.binding.public_key_id.clone(),
            signature,
            canonicalization_version: DEVICE_GPU_INVENTORY_CANONICALIZATION_VERSION.to_string(),
        },
        publication_fingerprint,
        binding: context.binding,
    })
}

fn submit_gpu_inventory(
    prepared: &PreparedInventory,
) -> Result<SubmitDeviceGpuInventoryResponse, ControlPlaneRequestError> {
    let enrollment = load_remote_enrollment().map_err(ControlPlaneRequestError::LocalState)?;
    let session = load_remote_session().map_err(ControlPlaneRequestError::LocalState)?;
    let current = PublicationBinding {
        control_plane_url: session.control_plane_url.clone(),
        provider_id: enrollment.provider_id.clone(),
        device_id: enrollment.device_id.clone(),
        session_id: session.session_id.clone(),
        public_key_id: enrollment.public_key_id.clone(),
    };
    ensure_submission_binding(prepared, &current).map_err(ControlPlaneRequestError::LocalState)?;
    let url = join_url(
        &session.control_plane_url,
        &format!("/v1/sessions/{}/gpu-inventory", session.session_id),
    );
    let mut response = ureq::post(&url)
        .header(
            "Authorization",
            &format!("Bearer {}", enrollment.credential),
        )
        .header("X-Burd-Session-Token", &session.resume_token)
        .header("X-Burd-Device-Id", &enrollment.device_id)
        .config()
        .timeout_global(Some(HTTP_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .send_json(&prepared.signed)
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
                .unwrap_or("control plane rejected GPU inventory")
                .to_string(),
        });
    }
    serde_json::from_value(
        value.map_err(|error| ControlPlaneRequestError::Contract(error.to_string()))?,
    )
    .map_err(|error| ControlPlaneRequestError::Contract(error.to_string()))
}

fn ensure_submission_binding(
    prepared: &PreparedInventory,
    current: &PublicationBinding,
) -> Result<(), String> {
    if &prepared.binding != current
        || prepared.signed.payload.provider_id != current.provider_id
        || prepared.signed.payload.device_id != current.device_id
        || prepared.signed.payload.session_id != current.session_id
        || prepared.signed.public_key_id != current.public_key_id
    {
        return Err("remote identity changed before GPU inventory submission".to_string());
    }
    Ok(())
}

fn already_published(
    last_published: Option<&(PublicationBinding, String)>,
    prepared: &PreparedInventory,
) -> bool {
    last_published.is_some_and(|(binding, fingerprint)| {
        binding == &prepared.binding && fingerprint == &prepared.publication_fingerprint
    })
}

fn request_failure_reason(error: &ControlPlaneRequestError) -> &'static str {
    match error {
        ControlPlaneRequestError::LocalState(_) => "local_state_changed",
        ControlPlaneRequestError::Transport(_) => "transport_error",
        ControlPlaneRequestError::Contract(_) => "response_contract_invalid",
        ControlPlaneRequestError::Rejected {
            status: 401 | 403, ..
        } => "authentication_rejected",
        ControlPlaneRequestError::Rejected { status, .. } if *status >= 500 => "server_error",
        ControlPlaneRequestError::Rejected { .. } => "request_rejected",
    }
}

async fn wait_for_retry_or_binding_change(
    expected: &PublicationBinding,
    duration: Duration,
    shutdown: &mut watch::Receiver<bool>,
) -> RetryWait {
    let deadline = Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return RetryWait::Elapsed;
        }
        if wait_for_duration_or_shutdown(remaining.min(STATE_POLL_INTERVAL), shutdown).await {
            return RetryWait::Shutdown;
        }
        match tokio::task::spawn_blocking(load_publication_binding).await {
            Ok(Ok(current)) if &current != expected => return RetryWait::BindingChanged,
            _ => {}
        }
    }
}

async fn wait_for_duration_or_shutdown(
    duration: Duration,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = wait_for_shutdown(shutdown) => true,
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

fn log_event(event: &str, reason_code: Option<&str>, inventory_hash: Option<&str>) {
    println!("{}", event_json(event, reason_code, inventory_hash));
}

fn event_json(
    event: &str,
    reason_code: Option<&str>,
    inventory_hash: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "event": event,
        "reason_code": reason_code,
        "inventory_hash": inventory_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_hardware::NvidiaGpuInventoryDevice;
    use burd_protocol::{generate_keypair, verify_message};

    fn binding(session_id: &str, public_key_id: &str) -> PublicationBinding {
        PublicationBinding {
            control_plane_url: "https://control.example".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: session_id.to_string(),
            public_key_id: public_key_id.to_string(),
        }
    }

    fn collection(devices: Vec<NvidiaGpuInventoryDevice>) -> NvidiaTelemetryCollection {
        NvidiaTelemetryCollection {
            collector: "test".to_string(),
            samples: Vec::new(),
            inventory: devices,
            warnings: Vec::new(),
        }
    }

    fn gpu(index: u32, uuid: &str) -> NvidiaGpuInventoryDevice {
        NvidiaGpuInventoryDevice {
            gpu_index: index,
            gpu_uuid: uuid.to_string(),
            pci_vendor_id: Some("10de".to_string()),
            pci_device_id: Some("2684".to_string()),
            vram_total_mib: 24_576,
        }
    }

    fn prepare(
        session_id: &str,
        public_key_id: &str,
        secret_key_base64: &str,
        devices: Vec<NvidiaGpuInventoryDevice>,
        observed_at: &str,
    ) -> PreparedInventory {
        prepare_inventory(
            SigningContext {
                binding: binding(session_id, public_key_id),
                hardware_fingerprint: "a".repeat(64),
                secret_key_base64: secret_key_base64.to_string(),
            },
            collection(devices),
            observed_at.to_string(),
        )
        .unwrap()
    }

    #[test]
    fn one_gpu_snapshot_is_complete_and_signature_verifies() {
        let keys = generate_keypair().unwrap();
        let prepared = prepare(
            "session_1",
            "key_1",
            &keys.secret_key_base64,
            vec![gpu(3, "GPU-A")],
            "2026-08-08T00:00:00Z",
        );
        validate_device_gpu_inventory_payload(&prepared.signed.payload).unwrap();
        assert_eq!(prepared.signed.payload.gpus.len(), 1);
        assert_eq!(prepared.signed.payload.gpus[0].gpu_index, 3);
        assert_eq!(prepared.signed.payload.gpus[0].backend, "cuda");
        assert_eq!(prepared.signed.payload.gpus[0].status, "active");
        let message = device_gpu_inventory_signature_message(
            &prepared.signed.payload,
            &prepared.signed.inventory_hash,
            &prepared.signed.public_key_id,
        )
        .unwrap();
        assert!(
            verify_message(
                &keys.public_key_base64,
                message.as_bytes(),
                &prepared.signed.signature,
            )
            .unwrap()
        );
    }

    #[test]
    fn empty_gpu_snapshot_is_complete_and_signature_verifies() {
        let keys = generate_keypair().unwrap();
        let prepared = prepare(
            "session_1",
            "key_1",
            &keys.secret_key_base64,
            Vec::new(),
            "2026-08-08T00:00:00Z",
        );
        validate_device_gpu_inventory_payload(&prepared.signed.payload).unwrap();
        assert!(prepared.signed.payload.gpus.is_empty());
        let message = device_gpu_inventory_signature_message(
            &prepared.signed.payload,
            &prepared.signed.inventory_hash,
            &prepared.signed.public_key_id,
        )
        .unwrap();
        assert!(
            verify_message(
                &keys.public_key_base64,
                message.as_bytes(),
                &prepared.signed.signature,
            )
            .unwrap()
        );
    }

    #[test]
    fn multi_gpu_order_and_publication_fingerprint_are_deterministic() {
        let keys = generate_keypair().unwrap();
        let first = prepare(
            "session_1",
            "key_1",
            &keys.secret_key_base64,
            vec![gpu(1, "GPU-B"), gpu(0, "GPU-A")],
            "2026-08-08T00:00:00Z",
        );
        let second = prepare(
            "session_1",
            "key_1",
            &keys.secret_key_base64,
            vec![gpu(0, "GPU-A"), gpu(1, "GPU-B")],
            "2026-08-08T00:01:00Z",
        );
        assert_eq!(
            first
                .signed
                .payload
                .gpus
                .iter()
                .map(|gpu| gpu.gpu_uuid.as_str())
                .collect::<Vec<_>>(),
            ["GPU-A", "GPU-B"]
        );
        assert_ne!(first.signed.inventory_hash, second.signed.inventory_hash);
        assert_eq!(
            first.publication_fingerprint,
            second.publication_fingerprint
        );
        let last_published = (first.binding.clone(), first.publication_fingerprint.clone());
        assert!(already_published(Some(&last_published), &second));
    }

    #[test]
    fn invalid_or_duplicate_gpu_identity_fails_closed() {
        let keys = generate_keypair().unwrap();
        for devices in [
            vec![gpu(0, "GPU-A"), gpu(1, "gpu-a")],
            vec![gpu(0, "GPU-A"), gpu(0, "GPU-B")],
            vec![gpu(0, "GPU\nsecret")],
        ] {
            let result = prepare_inventory(
                SigningContext {
                    binding: binding("session_1", "key_1"),
                    hardware_fingerprint: "a".repeat(64),
                    secret_key_base64: keys.secret_key_base64.clone(),
                },
                collection(devices),
                "2026-08-08T00:00:00Z".to_string(),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn added_removed_gpu_or_rotated_key_requires_a_new_publication() {
        let first_keys = generate_keypair().unwrap();
        let rotated_keys = generate_keypair().unwrap();
        let base = prepare(
            "session_1",
            "key_1",
            &first_keys.secret_key_base64,
            vec![gpu(0, "GPU-A")],
            "2026-08-08T00:00:00Z",
        );
        let added = prepare(
            "session_1",
            "key_1",
            &first_keys.secret_key_base64,
            vec![gpu(0, "GPU-A"), gpu(1, "GPU-B")],
            "2026-08-08T00:00:00Z",
        );
        let rotated = prepare(
            "session_1",
            "key_2",
            &rotated_keys.secret_key_base64,
            vec![gpu(0, "GPU-A")],
            "2026-08-08T00:00:00Z",
        );
        let empty = prepare(
            "session_1",
            "key_1",
            &first_keys.secret_key_base64,
            Vec::new(),
            "2026-08-08T00:00:00Z",
        );
        assert_ne!(base.publication_fingerprint, added.publication_fingerprint);
        assert_ne!(base.publication_fingerprint, empty.publication_fingerprint);
        assert_ne!(
            base.publication_fingerprint,
            rotated.publication_fingerprint
        );
        assert!(ensure_submission_binding(&base, &binding("session_1", "key_2")).is_err());
        assert!(ensure_submission_binding(&base, &binding("session_2", "key_1")).is_err());
    }

    fn empty_collector(_: u64) -> Result<NvidiaTelemetryCollection, String> {
        Ok(collection(Vec::new()))
    }

    fn unavailable_collector(_: u64) -> Result<NvidiaTelemetryCollection, String> {
        Err("nvidia-smi failed".to_string())
    }

    #[test]
    fn completed_empty_discovery_is_not_conflated_with_unavailable_probe() {
        assert!(matches!(
            discover_gpu_inventory(empty_collector, 0),
            GpuInventoryDiscovery::Empty(_)
        ));
        assert!(matches!(
            discover_gpu_inventory(unavailable_collector, 0),
            GpuInventoryDiscovery::Unavailable(error) if error == "nvidia-smi failed"
        ));
    }

    #[test]
    fn retry_backoff_is_bounded_and_resettable() {
        let mut backoff = RetryBackoff::default();
        assert_eq!(backoff.next_delay(), Duration::from_secs(2));
        assert_eq!(backoff.next_delay(), Duration::from_secs(4));
        for _ in 0..10 {
            assert!(backoff.next_delay() <= RETRY_MAX);
        }
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(2));
    }

    #[test]
    fn failure_log_metadata_never_contains_the_error_or_secrets() {
        let error = ControlPlaneRequestError::Transport(
            "Bearer secret-token private-key signature-value".to_string(),
        );
        let value = event_json(
            "gpu_inventory_submission_failed",
            Some(request_failure_reason(&error)),
            None,
        );
        let serialized = value.to_string().to_ascii_lowercase();
        for forbidden in ["secret-token", "private-key", "signature-value", "bearer"] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn request_failures_have_stable_retry_reason_codes() {
        assert_eq!(
            request_failure_reason(&ControlPlaneRequestError::Rejected {
                status: 401,
                code: "unauthorized".to_string(),
                message: "denied".to_string(),
            }),
            "authentication_rejected"
        );
        assert_eq!(
            request_failure_reason(&ControlPlaneRequestError::Rejected {
                status: 503,
                code: "unavailable".to_string(),
                message: "retry".to_string(),
            }),
            "server_error"
        );
        assert_eq!(
            request_failure_reason(&ControlPlaneRequestError::Transport("timeout".to_string())),
            "transport_error"
        );
    }

    fn panic_collector(_: u64) -> Result<NvidiaTelemetryCollection, String> {
        panic!("collector must not run after shutdown")
    }

    #[tokio::test]
    async fn shutdown_stops_worker_before_discovery() {
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);
        run_worker("test-agent".to_string(), panic_collector, shutdown_rx)
            .await
            .unwrap();
    }
}
