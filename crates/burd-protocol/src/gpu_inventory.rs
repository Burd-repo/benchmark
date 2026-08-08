use crate::{canonical_json, hash_canonical};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const DEVICE_GPU_INVENTORY_SCHEMA_VERSION: &str = "burd-device-gpu-inventory-v1";
pub const DEVICE_GPU_INVENTORY_CANONICALIZATION_VERSION: &str = "burd-json-c14n-v1";
pub const DEVICE_GPU_INVENTORY_SIGNATURE_DOMAIN: &str = "burd.device-gpu-inventory.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceGpuInventoryGpu {
    pub gpu_uuid: String,
    pub gpu_index: u32,
    pub backend: String,
    pub pci_vendor_id: String,
    pub pci_device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_total_mib: Option<u64>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceGpuInventoryPayload {
    pub schema_version: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub hardware_fingerprint: String,
    pub observed_at: String,
    pub gpus: Vec<DeviceGpuInventoryGpu>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedDeviceGpuInventory {
    pub payload: DeviceGpuInventoryPayload,
    pub inventory_hash: String,
    pub public_key_id: String,
    pub signature: String,
    pub canonicalization_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceGpuInventoryVerification {
    pub schema_version: String,
    pub inventory_hash_valid: bool,
    pub signature_valid: bool,
    pub session_bound: bool,
    pub fingerprint_bound: bool,
    pub active_key_bound: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceGpuInventoryRecord {
    pub inventory_row_id: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub schema_version: String,
    pub inventory_hash: String,
    pub public_key_id: String,
    pub canonicalization_version: String,
    pub gpu_uuid: String,
    pub gpu_index: u32,
    pub backend: String,
    pub pci_vendor_id: String,
    pub pci_device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_total_mib: Option<u64>,
    pub status: String,
    pub observed_at: String,
    pub server_received_at: String,
    pub verification: DeviceGpuInventoryVerification,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitDeviceGpuInventoryResponse {
    pub request_id: String,
    pub duplicate: bool,
    pub records: Vec<DeviceGpuInventoryRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListProviderDeviceGpuInventoryResponse {
    pub request_id: String,
    pub provider_id: String,
    pub records: Vec<DeviceGpuInventoryRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct DeviceGpuInventorySignatureClaims<'a> {
    domain: &'static str,
    inventory_hash: &'a str,
    provider_id: &'a str,
    device_id: &'a str,
    session_id: &'a str,
    hardware_fingerprint: &'a str,
    public_key_id: &'a str,
}

pub fn device_gpu_inventory_hash(payload: &DeviceGpuInventoryPayload) -> Result<String, String> {
    hash_canonical(payload)
}

pub fn device_gpu_inventory_signature_message(
    payload: &DeviceGpuInventoryPayload,
    inventory_hash: &str,
    public_key_id: &str,
) -> Result<String, String> {
    canonical_json(&DeviceGpuInventorySignatureClaims {
        domain: DEVICE_GPU_INVENTORY_SIGNATURE_DOMAIN,
        inventory_hash,
        provider_id: &payload.provider_id,
        device_id: &payload.device_id,
        session_id: &payload.session_id,
        hardware_fingerprint: &payload.hardware_fingerprint,
        public_key_id,
    })
}

pub fn validate_device_gpu_inventory_payload(
    payload: &DeviceGpuInventoryPayload,
) -> Result<(), String> {
    if payload.schema_version != DEVICE_GPU_INVENTORY_SCHEMA_VERSION {
        return Err("unsupported device GPU inventory schema version".to_string());
    }
    for (label, value) in [
        ("provider_id", payload.provider_id.as_str()),
        ("device_id", payload.device_id.as_str()),
        ("session_id", payload.session_id.as_str()),
        (
            "hardware_fingerprint",
            payload.hardware_fingerprint.as_str(),
        ),
    ] {
        if !safe_short_ascii(value, 256) {
            return Err(format!("device GPU inventory {label} is invalid"));
        }
    }
    if DateTime::parse_from_rfc3339(&payload.observed_at).is_err() {
        return Err("device GPU inventory observed_at is invalid".to_string());
    }
    if payload.gpus.is_empty() || payload.gpus.len() > 32 {
        return Err("device GPU inventory must contain between 1 and 32 GPUs".to_string());
    }
    let mut seen_gpu_uuids = HashSet::new();
    let mut seen_gpu_indices = HashSet::new();
    for gpu in &payload.gpus {
        if !safe_gpu_uuid(&gpu.gpu_uuid) {
            return Err("device GPU inventory gpu_uuid is invalid".to_string());
        }
        if !seen_gpu_uuids.insert(gpu.gpu_uuid.to_ascii_lowercase()) {
            return Err("device GPU inventory must not repeat GPU UUIDs".to_string());
        }
        if !seen_gpu_indices.insert(gpu.gpu_index) {
            return Err("device GPU inventory must not repeat GPU indices".to_string());
        }
        for (label, value) in [
            ("backend", gpu.backend.as_str()),
            ("pci_vendor_id", gpu.pci_vendor_id.as_str()),
            ("pci_device_id", gpu.pci_device_id.as_str()),
        ] {
            if !safe_short_ascii(value, 128) {
                return Err(format!("device GPU inventory {label} is invalid"));
            }
        }
        if !matches!(
            gpu.status.as_str(),
            "active" | "inactive" | "degraded" | "retired"
        ) {
            return Err(
                "device GPU inventory status must be active, inactive, degraded, or retired"
                    .to_string(),
            );
        }
        if gpu.vram_total_mib == Some(0) {
            return Err("device GPU inventory VRAM must be positive when present".to_string());
        }
    }
    Ok(())
}

fn safe_short_ascii(value: &str, maximum_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_len
        && value.is_ascii()
        && !value.chars().any(char::is_control)
}

fn safe_gpu_uuid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{generate_keypair, sign_message, verify_message};

    fn payload() -> DeviceGpuInventoryPayload {
        DeviceGpuInventoryPayload {
            schema_version: DEVICE_GPU_INVENTORY_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            observed_at: "2026-07-14T00:00:00Z".to_string(),
            gpus: vec![
                DeviceGpuInventoryGpu {
                    gpu_uuid: "GPU-1".to_string(),
                    gpu_index: 0,
                    backend: "cuda".to_string(),
                    pci_vendor_id: "10de".to_string(),
                    pci_device_id: "2684".to_string(),
                    vram_total_mib: Some(24_576),
                    status: "active".to_string(),
                },
                DeviceGpuInventoryGpu {
                    gpu_uuid: "GPU-2".to_string(),
                    gpu_index: 1,
                    backend: "cuda".to_string(),
                    pci_vendor_id: "10de".to_string(),
                    pci_device_id: "2684".to_string(),
                    vram_total_mib: Some(24_576),
                    status: "active".to_string(),
                },
            ],
        }
    }

    #[test]
    fn gpu_inventory_signature_binds_device_and_gpu_list() {
        let payload = payload();
        let hash = device_gpu_inventory_hash(&payload).unwrap();
        let message = device_gpu_inventory_signature_message(&payload, &hash, "key_1").unwrap();
        let keys = generate_keypair().unwrap();
        let signature = sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap();
        assert!(verify_message(&keys.public_key_base64, message.as_bytes(), &signature).unwrap());

        let mut changed = payload;
        changed.gpus[1].gpu_uuid = "GPU-3".to_string();
        let changed_hash = device_gpu_inventory_hash(&changed).unwrap();
        let changed_message =
            device_gpu_inventory_signature_message(&changed, &changed_hash, "key_1").unwrap();
        assert_ne!(hash, changed_hash);
        assert!(
            !verify_message(
                &keys.public_key_base64,
                changed_message.as_bytes(),
                &signature
            )
            .unwrap()
        );
    }

    #[test]
    fn payload_validation_rejects_duplicate_or_invalid_gpu_identity() {
        let mut payload = payload();
        validate_device_gpu_inventory_payload(&payload).unwrap();

        payload.gpus[1].gpu_uuid = "gpu-1".to_string();
        assert_eq!(
            validate_device_gpu_inventory_payload(&payload).unwrap_err(),
            "device GPU inventory must not repeat GPU UUIDs"
        );

        payload.gpus[1].gpu_uuid = "GPU\nsecret".to_string();
        assert_eq!(
            validate_device_gpu_inventory_payload(&payload).unwrap_err(),
            "device GPU inventory gpu_uuid is invalid"
        );
    }
}
