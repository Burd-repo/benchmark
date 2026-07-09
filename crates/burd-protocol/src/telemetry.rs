use crate::{canonical_json, hash_canonical};
use serde::{Deserialize, Serialize};

pub const TELEMETRY_SCHEMA_VERSION: &str = "burd-gpu-telemetry-v1";
pub const TELEMETRY_SIGNATURE_DOMAIN: &str = "burd.telemetry-batch.v1";
pub const TELEMETRY_CANONICALIZATION_VERSION: &str = "burd-json-c14n-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuProcessTelemetry {
    pub pid: u32,
    pub process_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_gpu_memory_mib: Option<u64>,
    pub process_kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuTelemetrySample {
    pub sample_sequence: u64,
    pub observed_at: String,
    pub gpu_uuid: String,
    pub gpu_name: String,
    pub pci_bus_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pci_vendor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pci_device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_capability: Option<String>,
    pub driver_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_driver_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_runtime_version: Option<String>,
    pub vram_total_mib: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_used_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_free_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_utilization_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_utilization_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_celsius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_draw_watts: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_limit_watts: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics_clock_mhz: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sm_clock_mhz: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_clock_mhz: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance_state: Option<String>,
    #[serde(default)]
    pub throttle_reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecc_corrected_errors: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ecc_uncorrected_errors: Option<u64>,
    #[serde(default)]
    pub processes: Vec<GpuProcessTelemetry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryBatchPayload {
    pub schema_version: String,
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub control_sequence: u64,
    pub sample_sequence_start: u64,
    pub sample_sequence_end: u64,
    pub hardware_fingerprint: String,
    pub collector: String,
    pub collected_at_start: String,
    pub collected_at_end: String,
    pub samples: Vec<GpuTelemetrySample>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedTelemetryBatch {
    pub payload: TelemetryBatchPayload,
    pub batch_hash: String,
    pub public_key_id: String,
    pub signature: String,
    pub canonicalization_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryBatchReceipt {
    pub request_id: String,
    pub batch_id: String,
    pub session_id: String,
    pub control_sequence_ack: u64,
    pub sample_sequence_end: u64,
    pub sample_count: usize,
    pub batch_hash: String,
    pub status: String,
    pub server_received_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatestTelemetryResponse {
    pub request_id: String,
    pub session_id: String,
    pub batch_id: String,
    pub batch_hash: String,
    pub server_received_at: String,
    pub samples: Vec<GpuTelemetrySample>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct TelemetrySignatureClaims<'a> {
    domain: &'static str,
    batch_hash: &'a str,
    provider_id: &'a str,
    device_id: &'a str,
    session_id: &'a str,
    control_sequence: u64,
    sample_sequence_start: u64,
    sample_sequence_end: u64,
    hardware_fingerprint: &'a str,
    public_key_id: &'a str,
}

pub fn telemetry_batch_hash(payload: &TelemetryBatchPayload) -> Result<String, String> {
    hash_canonical(payload)
}

pub fn telemetry_batch_signature_message(
    payload: &TelemetryBatchPayload,
    batch_hash: &str,
    public_key_id: &str,
) -> Result<String, String> {
    canonical_json(&TelemetrySignatureClaims {
        domain: TELEMETRY_SIGNATURE_DOMAIN,
        batch_hash,
        provider_id: &payload.provider_id,
        device_id: &payload.device_id,
        session_id: &payload.session_id,
        control_sequence: payload.control_sequence,
        sample_sequence_start: payload.sample_sequence_start,
        sample_sequence_end: payload.sample_sequence_end,
        hardware_fingerprint: &payload.hardware_fingerprint,
        public_key_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{generate_keypair, sign_message, verify_message};

    fn payload() -> TelemetryBatchPayload {
        TelemetryBatchPayload {
            schema_version: TELEMETRY_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            control_sequence: 4,
            sample_sequence_start: 1,
            sample_sequence_end: 1,
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            collector: "nvidia-smi-csv-v1".to_string(),
            collected_at_start: "2026-07-09T00:00:00Z".to_string(),
            collected_at_end: "2026-07-09T00:00:00Z".to_string(),
            samples: vec![GpuTelemetrySample {
                sample_sequence: 1,
                observed_at: "2026-07-09T00:00:00Z".to_string(),
                gpu_uuid: "GPU-test".to_string(),
                gpu_name: "NVIDIA RTX 4090".to_string(),
                pci_bus_id: "00000000:01:00.0".to_string(),
                pci_vendor_id: Some("10de".to_string()),
                pci_device_id: Some("2684".to_string()),
                compute_capability: Some("8.9".to_string()),
                driver_version: "576.80".to_string(),
                cuda_driver_version: Some("12.9".to_string()),
                cuda_runtime_version: None,
                vram_total_mib: 24564,
                vram_used_mib: Some(1024),
                vram_free_mib: Some(23540),
                gpu_utilization_percent: Some(20.0),
                memory_utilization_percent: Some(10.0),
                temperature_celsius: Some(45.0),
                power_draw_watts: Some(80.5),
                power_limit_watts: Some(450.0),
                graphics_clock_mhz: Some(2100),
                sm_clock_mhz: Some(2100),
                memory_clock_mhz: Some(10501),
                performance_state: Some("P2".to_string()),
                throttle_reasons: vec![],
                ecc_corrected_errors: None,
                ecc_uncorrected_errors: None,
                processes: vec![],
                container_id: None,
                job_id: None,
            }],
        }
    }

    #[test]
    fn telemetry_hash_and_signature_bind_session_and_sequences() {
        let payload = payload();
        let hash = telemetry_batch_hash(&payload).unwrap();
        let message = telemetry_batch_signature_message(&payload, &hash, "key_1").unwrap();
        let keys = generate_keypair().unwrap();
        let signature = sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap();
        assert!(verify_message(&keys.public_key_base64, message.as_bytes(), &signature).unwrap());

        let mut changed = payload;
        changed.control_sequence += 1;
        let changed_message = telemetry_batch_signature_message(&changed, &hash, "key_1").unwrap();
        assert!(
            !verify_message(
                &keys.public_key_base64,
                changed_message.as_bytes(),
                &signature
            )
            .unwrap()
        );
    }
}
