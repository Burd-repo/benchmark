use crate::actions::logs_summary;
use crate::earnings::{EarningsReport, estimate_earnings};
use crate::health::{UptimeSummary, detect_health_from_system};
use crate::pricing::{PricingReport, calculate_pricing};
use crate::report::{
    ReportRunOptions, generate_full_report_from_snapshot, load_latest_signed_report,
};
use crate::score::{ScoreReport, calculate_score};
use crate::verification::{ProviderVerification, verify_provider_from_reports};
use burd_hardware::{SystemReport, build_system_report, detect_specs};
use burd_llmfit::build_fit_report;
use burd_protocol::load_identity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurdProviderDetails {
    pub provider_id: String,
    pub machine_id: String,
    pub public_key: Option<String>,
    pub host_uri: String,
    pub created_at: Option<String>,
    pub last_check_date: String,
    pub is_online: bool,
    pub is_verified: bool,
    pub is_audited: bool,
    pub audit_status: String,
    pub location: ProviderLocation,
    pub hardware: ProviderHardware,
    pub gpu_models: Vec<GpuModelDetail>,
    pub uptime_1d: f64,
    pub uptime_7d: f64,
    pub uptime_30d: f64,
    pub uptime: UptimeSummary,
    pub stats: ProviderStats,
    pub pricing: PricingReport,
    pub score: ScoreReport,
    pub tier: String,
    pub active_jobs_future: u32,
    pub total_jobs_future: u32,
    pub estimated_earnings: EarningsReport,
    pub attributes: Vec<ProviderAttribute>,
    pub logs_summary: serde_json::Value,
    pub raw_report: serde_json::Value,
    pub verification: ProviderVerification,
    pub backend_verification_status_future: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderLocation {
    pub country: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHardware {
    pub cpu: String,
    pub architecture: String,
    pub memory_gb: f64,
    pub disk_free_gb: Option<f64>,
    pub backend: String,
    pub gpu_count: u32,
    pub vram_gb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_confidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuModelDetail {
    pub vendor: String,
    pub model: String,
    pub vram_gb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_confidence: Option<String>,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStats {
    pub cpu: ResourceStat,
    pub gpu: ResourceStat,
    pub memory: ResourceStat,
    pub storage: StorageStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStat {
    pub available: f64,
    pub active: f64,
    pub pending: f64,
    pub total: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    pub ephemeral: ResourceStat,
    pub persistent_future: ResourceStat,
    pub total: ResourceStat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAttribute {
    pub key: String,
    pub value: String,
}

pub fn build_provider_details(agent_version: &str, host_uri: &str) -> BurdProviderDetails {
    let identity_result = load_identity();
    let identity = identity_result.as_ref().ok();
    let specs = detect_specs();
    let system = build_system_report(&specs, agent_version);
    let fit = build_fit_report(&specs, Some(25));
    let score = calculate_score(&system, Some(&fit), None, None, None, None);
    let pricing = calculate_pricing(&system, &score);
    let earnings = estimate_earnings(&pricing);
    let health = detect_health_from_system(agent_version, &system);
    let verification = verify_provider_from_reports(
        identity_result.as_ref().map(|_| ()).map_err(Clone::clone),
        &system,
        &score,
        load_latest_signed_report(),
    );
    let raw_report = serde_json::to_value(generate_full_report_from_snapshot(
        ReportRunOptions::new(agent_version.to_string()),
        &system,
        &fit,
    ))
    .unwrap_or_else(|_| serde_json::json!({"error": "report serialization failed"}));

    let provider_id = identity
        .as_ref()
        .map(|config| config.provider_id.clone())
        .unwrap_or_else(|| "uninitialized".to_string());
    let machine_id = identity
        .as_ref()
        .map(|config| config.machine_id.clone())
        .unwrap_or_else(|| "uninitialized".to_string());
    let location = ProviderLocation {
        country: identity.as_ref().and_then(|config| config.country.clone()),
        city: identity.as_ref().and_then(|config| config.city.clone()),
        region: identity.as_ref().and_then(|config| config.region.clone()),
        timezone: None,
    };

    BurdProviderDetails {
        provider_id,
        machine_id,
        public_key: identity.as_ref().map(|config| config.public_key.clone()),
        host_uri: host_uri.to_string(),
        created_at: identity.as_ref().map(|config| config.created_at.clone()),
        last_check_date: health.last_seen_at.clone(),
        is_online: health.online,
        is_verified: verification.audit_status == "self_verified",
        is_audited: verification.audit_status == "burd_verified_future",
        audit_status: verification.audit_status.clone(),
        location,
        hardware: hardware_from_system(&system, health.disk_free_gb),
        gpu_models: gpu_models_from_system(&system),
        uptime_1d: health.uptime.uptime_1d,
        uptime_7d: health.uptime.uptime_7d,
        uptime_30d: health.uptime.uptime_30d,
        uptime: health.uptime,
        stats: stats_from_system(&system, health.disk_free_gb),
        pricing: pricing.clone(),
        tier: score.tier.clone(),
        score,
        active_jobs_future: 0,
        total_jobs_future: 0,
        estimated_earnings: earnings,
        attributes: attributes_from_system(&system),
        logs_summary: logs_summary().unwrap_or_else(|_| {
            serde_json::json!({
                "actions_total": 0,
                "logs_total": 0,
                "latest_action": null,
            })
        }),
        raw_report,
        verification,
        backend_verification_status_future: "not_connected".to_string(),
    }
}

fn hardware_from_system(system: &SystemReport, disk_free_gb: Option<f64>) -> ProviderHardware {
    ProviderHardware {
        cpu: system.cpu.clone(),
        architecture: system.architecture.clone(),
        memory_gb: system.ram_total_gb,
        disk_free_gb,
        backend: system.backend_detected.clone(),
        gpu_count: system.gpu_count,
        vram_gb: system.vram_total_gb.or(system.vram_per_gpu_gb),
        vram_source: system.vram_source.clone(),
        vram_confidence: system.vram_confidence.clone(),
    }
}

fn gpu_models_from_system(system: &SystemReport) -> Vec<GpuModelDetail> {
    system
        .gpus
        .iter()
        .map(|gpu| GpuModelDetail {
            vendor: gpu_vendor(&gpu.name),
            model: gpu.name.clone(),
            vram_gb: gpu.vram_gb,
            vram_source: gpu.vram_source.clone(),
            vram_confidence: gpu.vram_confidence.clone(),
            count: gpu.count,
        })
        .collect()
}

fn stats_from_system(system: &SystemReport, disk_free_gb: Option<f64>) -> ProviderStats {
    let cpu_total = system.cpu_cores as f64;
    let gpu_total = system.gpu_count as f64;
    let memory_total = system.ram_total_gb;
    let storage_available = disk_free_gb.unwrap_or(0.0);
    ProviderStats {
        cpu: ResourceStat {
            available: cpu_total,
            active: 0.0,
            pending: 0.0,
            total: cpu_total,
        },
        gpu: ResourceStat {
            available: gpu_total,
            active: 0.0,
            pending: 0.0,
            total: gpu_total,
        },
        memory: ResourceStat {
            available: system.ram_available_gb,
            active: 0.0,
            pending: 0.0,
            total: memory_total,
        },
        storage: StorageStats {
            ephemeral: ResourceStat {
                available: storage_available,
                active: 0.0,
                pending: 0.0,
                total: storage_available,
            },
            persistent_future: ResourceStat {
                available: 0.0,
                active: 0.0,
                pending: 0.0,
                total: 0.0,
            },
            total: ResourceStat {
                available: storage_available,
                active: 0.0,
                pending: 0.0,
                total: storage_available,
            },
        },
    }
}

fn attributes_from_system(system: &SystemReport) -> Vec<ProviderAttribute> {
    let mut attrs = vec![
        ProviderAttribute {
            key: "host".to_string(),
            value: "burd".to_string(),
        },
        ProviderAttribute {
            key: "hardware-cpu-arch".to_string(),
            value: system.architecture.clone(),
        },
        ProviderAttribute {
            key: "backend".to_string(),
            value: system.backend_detected.clone(),
        },
    ];
    if let Some(gpu) = system.primary_gpu_name.clone() {
        attrs.push(ProviderAttribute {
            key: "hardware-gpu-model".to_string(),
            value: gpu,
        });
    }
    if let Some(vram) = system.vram_total_gb.or(system.vram_per_gpu_gb) {
        attrs.push(ProviderAttribute {
            key: "hardware-gpu-vram-gb".to_string(),
            value: format!("{vram:.1}"),
        });
    }
    if let Some(source) = system.vram_source.clone() {
        attrs.push(ProviderAttribute {
            key: "hardware-gpu-vram-source".to_string(),
            value: source,
        });
    }
    if let Some(confidence) = system.vram_confidence.clone() {
        attrs.push(ProviderAttribute {
            key: "hardware-gpu-vram-confidence".to_string(),
            value: confidence,
        });
    }
    attrs
}

fn gpu_vendor(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("nvidia")
        || lower.contains("rtx")
        || lower.contains("a100")
        || lower.contains("h100")
    {
        "nvidia".to_string()
    } else if lower.contains("amd") || lower.contains("radeon") {
        "amd".to_string()
    } else if lower.contains("apple") {
        "apple".to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_stats_have_future_job_defaults() {
        let system = SystemReport {
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
            cpu: "cpu".to_string(),
            cpu_cores: 8,
            ram_total_gb: 32.0,
            ram_available_gb: 16.0,
            gpus: vec![],
            gpu_count: 1,
            primary_gpu_name: Some("NVIDIA RTX 4090".to_string()),
            vram_per_gpu_gb: Some(24.0),
            vram_total_gb: Some(24.0),
            vram_source: None,
            vram_confidence: None,
            backend_detected: "CUDA".to_string(),
            cuda_available: true,
            rocm_available: false,
            nvidia_driver: None,
            amd_driver: None,
            container_detected: false,
            vm_detected: false,
            timestamp: "2026-06-08T00:00:00Z".to_string(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: "test".to_string(),
        };
        let stats = stats_from_system(&system, Some(100.0));
        assert_eq!(stats.gpu.active, 0.0);
        assert_eq!(stats.gpu.total, 1.0);
    }

    #[test]
    fn provider_preserves_vram_source_and_confidence() {
        let mut system = crate::test_fixtures::system_report();
        system.vram_source = Some("vulkan_device_memory".to_string());
        system.vram_confidence = Some("detected".to_string());
        system.gpus[0].vram_source = system.vram_source.clone();
        system.gpus[0].vram_confidence = system.vram_confidence.clone();

        let hardware = hardware_from_system(&system, None);
        let gpu_models = gpu_models_from_system(&system);

        assert_eq!(hardware.vram_gb, Some(24.0));
        assert_eq!(
            hardware.vram_source.as_deref(),
            Some("vulkan_device_memory")
        );
        assert_eq!(hardware.vram_confidence.as_deref(), Some("detected"));
        assert_eq!(
            gpu_models[0].vram_source.as_deref(),
            Some("vulkan_device_memory")
        );
        assert_eq!(gpu_models[0].vram_confidence.as_deref(), Some("detected"));
    }
}
