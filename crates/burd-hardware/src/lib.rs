use chrono::Utc;
use llmfit_core::hardware::{GpuBackend, SystemSpecs};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::process::Command;
pub mod telemetry;
pub use telemetry::{
    NVIDIA_SMI_COLLECTOR_VERSION, NvidiaGpuInventoryDevice, NvidiaTelemetryCollection,
    collect_nvidia_telemetry,
};

pub const BENCHMARK_VERSION: &str = "2026.06-mvp";
pub const HARDWARE_FINGERPRINT_VERSION: &str = "burd-hardware-fingerprint-v1";
pub const MARKETPLACE_GPU_POLICY: &str = "nvidia_cuda_only_mvp";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuReport {
    pub name: String,
    pub vram_gb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_confidence: Option<String>,
    pub backend: String,
    pub count: u32,
    pub unified_memory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemReport {
    pub os: String,
    pub architecture: String,
    pub cpu: String,
    pub cpu_cores: usize,
    pub ram_total_gb: f64,
    pub ram_available_gb: f64,
    pub gpus: Vec<GpuReport>,
    pub gpu_count: u32,
    pub primary_gpu_name: Option<String>,
    pub vram_per_gpu_gb: Option<f64>,
    pub vram_total_gb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_confidence: Option<String>,
    pub backend_detected: String,
    pub cuda_available: bool,
    pub rocm_available: bool,
    pub nvidia_driver: Option<String>,
    pub amd_driver: Option<String>,
    pub container_detected: bool,
    pub vm_detected: bool,
    pub timestamp: String,
    pub agent_version: String,
    pub benchmark_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareFingerprintGpu {
    pub name: String,
    pub vendor: String,
    pub count: u32,
    pub vram_gb: Option<f64>,
    pub vram_source: Option<String>,
    pub vram_confidence: Option<String>,
    pub backend: String,
    pub unified_memory: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareFingerprintPayload {
    pub version: String,
    pub os: String,
    pub architecture: String,
    pub cpu_name: String,
    pub cpu_cores: usize,
    pub ram_total_gb: f64,
    pub gpus: Vec<HardwareFingerprintGpu>,
    pub gpu_count: u32,
    pub primary_gpu_name: Option<String>,
    pub vram_total_gb: Option<f64>,
    pub vram_source: Option<String>,
    pub vram_confidence: Option<String>,
    pub backend_detected: String,
    pub cuda_available: bool,
    pub rocm_available: bool,
    pub vulkan_available: bool,
    pub nvidia_driver: Option<String>,
    pub amd_driver: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketplaceGpuPolicy {
    pub marketplace_eligible: bool,
    pub eligibility_level: String,
    pub gpu_policy: String,
    pub requires_nvidia: bool,
    pub requires_cuda: bool,
    pub requires_detected_vram: bool,
    pub minimum_class: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareFingerprintReport {
    pub hardware_fingerprint: String,
    pub algorithm: String,
    pub canonicalization_version: String,
    pub payload: HardwareFingerprintPayload,
    pub marketplace_policy: MarketplaceGpuPolicy,
}

pub fn detect_system_report(agent_version: &str) -> SystemReport {
    let specs = SystemSpecs::detect();
    build_system_report(&specs, agent_version)
}

pub fn build_system_report(specs: &SystemSpecs, agent_version: &str) -> SystemReport {
    let gpus = specs
        .gpus
        .iter()
        .map(|gpu| GpuReport {
            name: gpu.name.clone(),
            vram_gb: gpu.vram_gb.map(round2),
            vram_source: gpu.vram_source.map(|source| source.label().to_string()),
            vram_confidence: gpu
                .vram_confidence
                .map(|confidence| confidence.label().to_string()),
            backend: gpu.backend.label().to_string(),
            count: gpu.count,
            unified_memory: gpu.unified_memory,
        })
        .collect();
    let primary = specs.gpus.first();

    SystemReport {
        os: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        cpu: specs.cpu_name.clone(),
        cpu_cores: specs.total_cpu_cores,
        ram_total_gb: round2(specs.total_ram_gb),
        ram_available_gb: round2(specs.available_ram_gb),
        gpus,
        gpu_count: specs.gpu_count,
        primary_gpu_name: specs.gpu_name.clone(),
        vram_per_gpu_gb: specs.gpu_vram_gb.map(round2),
        vram_total_gb: specs.total_gpu_vram_gb.map(round2),
        vram_source: primary
            .and_then(|gpu| gpu.vram_source)
            .map(|source| source.label().to_string()),
        vram_confidence: primary
            .and_then(|gpu| gpu.vram_confidence)
            .map(|confidence| confidence.label().to_string()),
        backend_detected: specs.backend.label().to_string(),
        cuda_available: specs.backend == GpuBackend::Cuda || command_exists("nvidia-smi"),
        rocm_available: specs.backend == GpuBackend::Rocm || command_exists("rocm-smi"),
        nvidia_driver: detect_nvidia_driver(),
        amd_driver: detect_amd_driver(),
        container_detected: detect_container(),
        vm_detected: detect_vm(&specs.cpu_name),
        timestamp: Utc::now().to_rfc3339(),
        agent_version: agent_version.to_string(),
        benchmark_version: BENCHMARK_VERSION.to_string(),
    }
}

pub fn detect_specs() -> SystemSpecs {
    SystemSpecs::detect()
}

pub fn build_hardware_fingerprint_report(system: &SystemReport) -> HardwareFingerprintReport {
    let payload = hardware_fingerprint_payload(system);
    let canonical = serde_json::to_vec(&payload).expect("hardware fingerprint payload serializes");
    let digest = Sha256::digest(canonical);
    HardwareFingerprintReport {
        hardware_fingerprint: format!("sha256:{}", hex_encode(&digest)),
        algorithm: "sha256".to_string(),
        canonicalization_version: HARDWARE_FINGERPRINT_VERSION.to_string(),
        payload,
        marketplace_policy: evaluate_marketplace_gpu_policy(system),
    }
}

pub fn hardware_fingerprint(system: &SystemReport) -> String {
    build_hardware_fingerprint_report(system).hardware_fingerprint
}

pub fn evaluate_marketplace_gpu_policy(system: &SystemReport) -> MarketplaceGpuPolicy {
    let mut reasons = Vec::new();
    let gpu_names = gpu_names(system);
    let vendors: Vec<String> = gpu_names.iter().map(|name| gpu_vendor(name)).collect();
    let has_gpu = system.gpu_count > 0 && !gpu_names.is_empty();
    let all_nvidia = has_gpu && vendors.iter().all(|vendor| vendor == "nvidia");
    let supported_class = has_gpu && gpu_names.iter().all(|name| is_supported_nvidia_gpu(name));
    let cuda_backend = system.cuda_available
        && system
            .backend_detected
            .to_ascii_lowercase()
            .contains("cuda")
        && system
            .gpus
            .iter()
            .all(|gpu| gpu.backend.to_ascii_lowercase().contains("cuda"));
    let detected_vram = system
        .vram_total_gb
        .or(system.vram_per_gpu_gb)
        .is_some_and(|vram| vram > 0.0)
        && system.vram_source.is_some()
        && system
            .vram_confidence
            .as_deref()
            .is_some_and(|confidence| confidence.eq_ignore_ascii_case("detected"));

    if !has_gpu {
        reasons.push("CPU-only providers are not eligible for the marketplace MVP".to_string());
    } else if !all_nvidia {
        reasons.push("Marketplace MVP requires NVIDIA CUDA GPUs".to_string());
    } else if !supported_class {
        reasons.push(
            "NVIDIA GPU is below or outside the supported RTX 30xx+ or datacenter policy"
                .to_string(),
        );
    }

    if has_gpu && !cuda_backend {
        reasons.push("CUDA backend is required for the marketplace MVP".to_string());
    }
    if has_gpu && !detected_vram {
        reasons.push(
            "Marketplace MVP requires detected VRAM with reliable source and confidence"
                .to_string(),
        );
    }

    let marketplace_eligible =
        has_gpu && all_nvidia && supported_class && cuda_backend && detected_vram;
    let eligibility_level = if marketplace_eligible {
        "marketplace_eligible"
    } else if has_gpu
        && (!all_nvidia
            || !cuda_backend
            || system.rocm_available
            || system
                .backend_detected
                .to_ascii_lowercase()
                .contains("vulkan"))
    {
        "local_diagnostic_only"
    } else {
        "not_eligible"
    };

    MarketplaceGpuPolicy {
        marketplace_eligible,
        eligibility_level: eligibility_level.to_string(),
        gpu_policy: MARKETPLACE_GPU_POLICY.to_string(),
        requires_nvidia: true,
        requires_cuda: true,
        requires_detected_vram: true,
        minimum_class: "rtx_30xx_or_datacenter".to_string(),
        reasons,
    }
}

pub fn gpu_vendor(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.contains("nvidia")
        || lower.contains("geforce")
        || lower.contains("quadro")
        || lower.contains("tesla")
        || lower.contains("rtx")
    {
        "nvidia".to_string()
    } else if lower.contains("amd") || lower.contains("radeon") {
        "amd".to_string()
    } else if lower.contains("intel") || lower.contains("arc ") {
        "intel".to_string()
    } else if lower.contains("apple") {
        "apple".to_string()
    } else {
        "unknown".to_string()
    }
}

fn hardware_fingerprint_payload(system: &SystemReport) -> HardwareFingerprintPayload {
    let mut gpus: Vec<HardwareFingerprintGpu> = system
        .gpus
        .iter()
        .map(|gpu| HardwareFingerprintGpu {
            name: normalize(&gpu.name),
            vendor: gpu_vendor(&gpu.name),
            count: gpu.count,
            vram_gb: gpu.vram_gb.map(round2),
            vram_source: gpu.vram_source.as_deref().map(normalize),
            vram_confidence: gpu.vram_confidence.as_deref().map(normalize),
            backend: normalize(&gpu.backend),
            unified_memory: gpu.unified_memory,
        })
        .collect();
    gpus.sort_by(|left, right| {
        (
            left.vendor.as_str(),
            left.name.as_str(),
            left.backend.as_str(),
            left.count,
        )
            .cmp(&(
                right.vendor.as_str(),
                right.name.as_str(),
                right.backend.as_str(),
                right.count,
            ))
    });

    HardwareFingerprintPayload {
        version: HARDWARE_FINGERPRINT_VERSION.to_string(),
        os: normalize(&system.os),
        architecture: normalize(&system.architecture),
        cpu_name: normalize(&system.cpu),
        cpu_cores: system.cpu_cores,
        ram_total_gb: round2(system.ram_total_gb),
        gpus,
        gpu_count: system.gpu_count,
        primary_gpu_name: system.primary_gpu_name.as_deref().map(normalize),
        vram_total_gb: system.vram_total_gb.or(system.vram_per_gpu_gb).map(round2),
        vram_source: system.vram_source.as_deref().map(normalize),
        vram_confidence: system.vram_confidence.as_deref().map(normalize),
        backend_detected: normalize(&system.backend_detected),
        cuda_available: system.cuda_available,
        rocm_available: system.rocm_available,
        vulkan_available: system
            .backend_detected
            .to_ascii_lowercase()
            .contains("vulkan")
            || system
                .gpus
                .iter()
                .any(|gpu| gpu.backend.to_ascii_lowercase().contains("vulkan")),
        nvidia_driver: system.nvidia_driver.as_deref().map(normalize),
        amd_driver: system.amd_driver.as_deref().map(normalize),
    }
}

fn gpu_names(system: &SystemReport) -> Vec<String> {
    if !system.gpus.is_empty() {
        system.gpus.iter().map(|gpu| gpu.name.clone()).collect()
    } else {
        system.primary_gpu_name.iter().cloned().collect()
    }
}

fn is_supported_nvidia_gpu(name: &str) -> bool {
    if gpu_vendor(name) != "nvidia" {
        return false;
    }

    let tokens: Vec<String> = name
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_uppercase())
        .collect();
    let datacenter = [
        "T4", "A10", "A10G", "A30", "A40", "A100", "L4", "L40", "L40S", "H100", "H200", "B200",
    ];
    if tokens
        .iter()
        .any(|token| datacenter.contains(&token.as_str()))
    {
        return true;
    }

    tokens.windows(2).any(|window| {
        window[0] == "RTX"
            && window[1].len() == 4
            && window[1]
                .chars()
                .all(|character| character.is_ascii_digit())
            && window[1]
                .chars()
                .next()
                .is_some_and(|generation| generation >= '3')
    })
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn detect_nvidia_driver() -> Option<String> {
    let output = Command::new("nvidia-smi")
        .arg("--query-gpu=driver_version")
        .arg("--format=csv,noheader")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn detect_amd_driver() -> Option<String> {
    let output = Command::new("rocm-smi")
        .arg("--showdriverversion")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && line.to_lowercase().contains("driver"))
        .map(ToOwned::to_owned)
}

fn command_exists(command: &str) -> bool {
    Command::new(command).arg("--help").output().is_ok()
}

fn detect_container() -> bool {
    std::env::var("container").is_ok()
        || std::env::var("DOTNET_RUNNING_IN_CONTAINER").is_ok()
        || std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .map(|value| value.contains("docker") || value.contains("containerd"))
            .unwrap_or(false)
}

fn detect_vm(cpu_name: &str) -> bool {
    let cpu_hint = cpu_name.to_lowercase();
    cpu_hint.contains("hyper-v")
        || cpu_hint.contains("kvm")
        || cpu_hint.contains("vmware")
        || cpu_hint.contains("virtualbox")
        || cpu_hint.contains("qemu")
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(gpu_name: Option<&str>, backend: &str, cuda_available: bool) -> SystemReport {
        let gpus = gpu_name
            .map(|name| {
                vec![GpuReport {
                    name: name.to_string(),
                    vram_gb: Some(24.0),
                    vram_source: Some("nvidia_smi".to_string()),
                    vram_confidence: Some("detected".to_string()),
                    backend: backend.to_string(),
                    count: 1,
                    unified_memory: false,
                }]
            })
            .unwrap_or_default();
        SystemReport {
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
            cpu: "Burd Test CPU".to_string(),
            cpu_cores: 16,
            ram_total_gb: 64.0,
            ram_available_gb: 48.0,
            gpu_count: if gpu_name.is_some() { 1 } else { 0 },
            primary_gpu_name: gpu_name.map(ToOwned::to_owned),
            vram_per_gpu_gb: gpu_name.map(|_| 24.0),
            vram_total_gb: gpu_name.map(|_| 24.0),
            vram_source: gpu_name.map(|_| "nvidia_smi".to_string()),
            vram_confidence: gpu_name.map(|_| "detected".to_string()),
            gpus,
            backend_detected: backend.to_string(),
            cuda_available,
            rocm_available: backend.eq_ignore_ascii_case("rocm"),
            nvidia_driver: gpu_name
                .filter(|name| gpu_vendor(name) == "nvidia")
                .map(|_| "555.42".to_string()),
            amd_driver: gpu_name
                .filter(|name| gpu_vendor(name) == "amd")
                .map(|_| "6.1".to_string()),
            container_detected: false,
            vm_detected: false,
            timestamp: "2026-06-08T00:00:00Z".to_string(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: BENCHMARK_VERSION.to_string(),
        }
    }

    #[test]
    fn report_serializes_to_json() {
        let report = SystemReport {
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
            cpu: "test".to_string(),
            cpu_cores: 8,
            ram_total_gb: 32.0,
            ram_available_gb: 24.0,
            gpus: vec![],
            gpu_count: 0,
            primary_gpu_name: None,
            vram_per_gpu_gb: None,
            vram_total_gb: None,
            vram_source: None,
            vram_confidence: None,
            backend_detected: "CPU (x86)".to_string(),
            cuda_available: false,
            rocm_available: false,
            nvidia_driver: None,
            amd_driver: None,
            container_detected: false,
            vm_detected: false,
            timestamp: "2026-06-08T00:00:00Z".to_string(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: BENCHMARK_VERSION.to_string(),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("ram_total_gb"));
    }

    #[test]
    fn vm_detection_uses_existing_cpu_name() {
        assert!(detect_vm("Virtual CPU (KVM)"));
        assert!(!detect_vm("AMD Ryzen 9 7950X"));
    }

    #[test]
    fn fingerprint_is_stable_and_ignores_runtime_only_fields() {
        let system = fixture(Some("NVIDIA GeForce RTX 4090"), "CUDA", true);
        let mut changed = system.clone();
        changed.timestamp = "2026-06-09T00:00:00Z".to_string();
        changed.ram_available_gb = 8.0;
        changed.container_detected = true;
        changed.vm_detected = true;

        assert_eq!(
            hardware_fingerprint(&system),
            hardware_fingerprint(&changed)
        );
    }

    #[test]
    fn fingerprint_changes_for_gpu_vram_and_cuda_changes() {
        let system = fixture(Some("NVIDIA GeForce RTX 4090"), "CUDA", true);
        let baseline = hardware_fingerprint(&system);

        let mut gpu_changed = system.clone();
        gpu_changed.primary_gpu_name = Some("NVIDIA GeForce RTX 3060".to_string());
        gpu_changed.gpus[0].name = "NVIDIA GeForce RTX 3060".to_string();
        assert_ne!(baseline, hardware_fingerprint(&gpu_changed));

        let mut vram_changed = system.clone();
        vram_changed.vram_total_gb = Some(12.0);
        vram_changed.vram_per_gpu_gb = Some(12.0);
        vram_changed.gpus[0].vram_gb = Some(12.0);
        assert_ne!(baseline, hardware_fingerprint(&vram_changed));

        let mut cuda_changed = system;
        cuda_changed.cuda_available = false;
        assert_ne!(baseline, hardware_fingerprint(&cuda_changed));
    }

    #[test]
    fn marketplace_policy_accepts_supported_nvidia_cuda_with_detected_vram() {
        for gpu in [
            "NVIDIA GeForce RTX 3060",
            "NVIDIA GeForce RTX 4090",
            "NVIDIA A100",
            "NVIDIA H100",
        ] {
            let policy = evaluate_marketplace_gpu_policy(&fixture(Some(gpu), "CUDA", true));
            assert!(policy.marketplace_eligible, "{gpu}: {:?}", policy.reasons);
            assert_eq!(policy.eligibility_level, "marketplace_eligible");
        }
    }

    #[test]
    fn marketplace_policy_keeps_unsupported_backends_as_local_diagnostic_only() {
        for (gpu, backend) in [
            ("AMD Radeon RX 7900 XTX", "ROCm"),
            ("AMD Radeon RX 7900 XTX", "Vulkan"),
            ("NVIDIA GeForce RTX 4090", "Vulkan"),
        ] {
            let policy = evaluate_marketplace_gpu_policy(&fixture(Some(gpu), backend, false));
            assert!(!policy.marketplace_eligible);
            assert_eq!(policy.eligibility_level, "local_diagnostic_only");
        }
    }

    #[test]
    fn marketplace_policy_rejects_cpu_only_and_unreliable_vram() {
        let cpu = evaluate_marketplace_gpu_policy(&fixture(None, "CPU (x86)", false));
        assert!(!cpu.marketplace_eligible);
        assert_eq!(cpu.eligibility_level, "not_eligible");

        let mut estimated = fixture(Some("NVIDIA GeForce RTX 4090"), "CUDA", true);
        estimated.vram_source = Some("known_gpu_table".to_string());
        estimated.vram_confidence = Some("estimated".to_string());
        let policy = evaluate_marketplace_gpu_policy(&estimated);
        assert!(!policy.marketplace_eligible);
        assert_eq!(policy.eligibility_level, "not_eligible");
    }
}
