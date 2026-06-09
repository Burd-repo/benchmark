use chrono::Utc;
use llmfit_core::hardware::{GpuBackend, SystemSpecs};
use serde::{Deserialize, Serialize};
use std::process::Command;

pub const BENCHMARK_VERSION: &str = "2026.06-mvp";

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
