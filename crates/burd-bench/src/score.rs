use crate::disk::DiskBenchmarkReport;
use crate::llm::LlmBenchmarkReport;
use crate::network::NetworkBenchmarkReport;
use crate::stability::StabilityBenchmarkReport;
use burd_hardware::SystemReport;
use burd_llmfit::FitReport;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreReport {
    pub burd_compute_score: f64,
    pub tier: String,
    pub eligible: bool,
    pub recommended_workloads: Vec<String>,
    pub not_recommended_workloads: Vec<String>,
    pub suggested_price_brl_hour: f64,
    pub price_basis: String,
    pub prices_are_demonstrative: bool,
    pub components: ScoreComponents,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreComponents {
    pub llm_benchmark: f64,
    pub vram_capacity: f64,
    pub stability: f64,
    pub network: f64,
    pub disk: f64,
    pub verification: f64,
}

pub fn calculate_score(
    system: &SystemReport,
    fit: Option<&FitReport>,
    llm: Option<&LlmBenchmarkReport>,
    stability: Option<&StabilityBenchmarkReport>,
    network: Option<&NetworkBenchmarkReport>,
    disk: Option<&DiskBenchmarkReport>,
) -> ScoreReport {
    let mut warnings = Vec::new();
    let vram = system
        .vram_total_gb
        .or(system.vram_per_gpu_gb)
        .unwrap_or(0.0);

    let llm_component = if let Some(llm) = llm {
        if llm.passed {
            normalize_tps(llm.avg_tps)
        } else {
            warnings
                .push("real LLM benchmark failed; score uses conservative fallback".to_string());
            15.0
        }
    } else if let Some(fit) = fit {
        warnings.push("real LLM benchmark unavailable; score uses llmfit TPS estimate".to_string());
        let estimated = fit
            .models
            .first()
            .map(|model| model.estimated_tps)
            .unwrap_or(0.0);
        (normalize_tps(estimated) * 0.75).min(75.0)
    } else {
        warnings.push("fit report unavailable; LLM component is minimal".to_string());
        10.0
    };

    let vram_component = vram_score(vram);
    let stability_component = stability.map(stability_score).unwrap_or_else(|| {
        warnings.push("stability benchmark unavailable; using neutral stability score".to_string());
        60.0
    });
    let network_component = network.map(network_score).unwrap_or_else(|| {
        warnings.push("network benchmark unavailable; using neutral network score".to_string());
        60.0
    });
    let disk_component = disk.map(disk_score).unwrap_or_else(|| {
        warnings.push("disk benchmark unavailable; using neutral disk score".to_string());
        60.0
    });
    let verification_component = verification_score(system);

    let components = ScoreComponents {
        llm_benchmark: round1(llm_component),
        vram_capacity: round1(vram_component),
        stability: round1(stability_component),
        network: round1(network_component),
        disk: round1(disk_component),
        verification: round1(verification_component),
    };

    let score = round1(
        components.llm_benchmark * 0.40
            + components.vram_capacity * 0.20
            + components.stability * 0.15
            + components.network * 0.10
            + components.disk * 0.10
            + components.verification * 0.05,
    );
    let tier = tier_for_score(score).to_string();
    let critical_failure = stability.map(|value| !value.passed).unwrap_or(false)
        || llm
            .map(|value| !value.passed && value.avg_tps == 0.0)
            .unwrap_or(false);

    let recommended_workloads = fit
        .map(|fit| fit.recommended_workloads.clone())
        .unwrap_or_else(|| burd_llmfit::workload_summary_from_vram(vram, system.gpu_count > 0).0);
    let not_recommended_workloads = fit
        .map(|fit| fit.not_recommended_workloads.clone())
        .unwrap_or_else(|| burd_llmfit::workload_summary_from_vram(vram, system.gpu_count > 0).1);

    let (price, basis) = suggested_price(system, score, llm.map(|value| value.avg_tps));

    ScoreReport {
        burd_compute_score: score,
        tier,
        eligible: score >= 40.0 && !critical_failure,
        recommended_workloads,
        not_recommended_workloads,
        suggested_price_brl_hour: price,
        price_basis: basis,
        prices_are_demonstrative: true,
        components,
        warnings,
        notes: vec![
            "Score weights: 40% LLM, 20% VRAM, 15% stability, 10% network, 10% disk, 5% verification.".to_string(),
            "Prices are demonstrative until Burd backend policy and marketplace demand are connected.".to_string(),
        ],
    }
}

pub fn tier_for_score(score: f64) -> &'static str {
    match score {
        value if value < 40.0 => "Not Eligible",
        value if value < 60.0 => "Burd Basic",
        value if value < 75.0 => "Burd Plus",
        value if value < 90.0 => "Burd Pro",
        value if value < 97.0 => "Burd Max",
        _ => "Burd Enterprise",
    }
}

fn normalize_tps(tps: f64) -> f64 {
    if tps <= 0.0 {
        0.0
    } else if tps < 5.0 {
        20.0
    } else {
        (tps / 120.0 * 100.0).clamp(20.0, 100.0)
    }
}

fn vram_score(vram: f64) -> f64 {
    match vram {
        value if value >= 80.0 => 100.0,
        value if value >= 48.0 => 92.0,
        value if value >= 24.0 => 82.0,
        value if value >= 16.0 => 68.0,
        value if value >= 12.0 => 58.0,
        value if value >= 8.0 => 45.0,
        value if value > 0.0 => 25.0,
        _ => 10.0,
    }
}

fn stability_score(report: &StabilityBenchmarkReport) -> f64 {
    if !report.passed {
        return 30.0;
    }
    (100.0 - report.performance_drop_pct * 2.0 - report.failed_runs as f64 * 10.0)
        .clamp(40.0, 100.0)
}

fn network_score(report: &NetworkBenchmarkReport) -> f64 {
    if !report.passed {
        return 35.0;
    }
    let latency_penalty = report.avg_latency_ms.unwrap_or(300.0) / 4.0;
    let jitter_penalty = report.jitter_ms.unwrap_or(0.0) / 3.0;
    (100.0 - latency_penalty - jitter_penalty - report.loss_pct).clamp(40.0, 100.0)
}

fn disk_score(report: &DiskBenchmarkReport) -> f64 {
    if !report.passed {
        return 35.0;
    }
    let read = (report.sequential_read_mb_s / 1500.0 * 50.0).min(50.0);
    let write = (report.sequential_write_mb_s / 1000.0 * 50.0).min(50.0);
    (read + write).clamp(40.0, 100.0)
}

fn verification_score(system: &SystemReport) -> f64 {
    let mut score: f64 = 20.0;
    if system.gpu_count > 0 {
        score += 35.0;
    }
    if system.cuda_available || system.rocm_available {
        score += 25.0;
    }
    if system.nvidia_driver.is_some() || system.amd_driver.is_some() {
        score += 10.0;
    }
    if !system.container_detected {
        score += 5.0;
    }
    if !system.vm_detected {
        score += 5.0;
    }
    score.min(100.0)
}

fn suggested_price(system: &SystemReport, score: f64, avg_tps: Option<f64>) -> (f64, String) {
    let gpu = system
        .primary_gpu_name
        .as_deref()
        .unwrap_or("unknown")
        .to_lowercase();
    let base = if gpu.contains("3060") && gpu.contains("12") {
        2.90
    } else if gpu.contains("3090") {
        4.90
    } else if gpu.contains("4090") {
        6.90
    } else if gpu.contains("a100") {
        34.90
    } else if gpu.contains("h100") {
        59.90
    } else {
        let vram = system
            .vram_total_gb
            .or(system.vram_per_gpu_gb)
            .unwrap_or(0.0);
        match vram {
            value if value >= 80.0 => 34.90,
            value if value >= 48.0 => 14.90,
            value if value >= 24.0 => 5.90,
            value if value >= 12.0 => 2.90,
            value if value >= 8.0 => 1.90,
            _ => 0.90,
        }
    };
    let score_factor = (0.75 + score / 400.0).clamp(0.75, 1.0);
    let tps_factor = avg_tps
        .map(|tps| (0.85 + normalize_tps(tps) / 1000.0).clamp(0.85, 1.0))
        .unwrap_or(0.9);
    let price = round2(base * score_factor * tps_factor);
    (
        price,
        format!(
            "demonstrative base from GPU/VRAM adjusted by score ({score:.1}) and benchmark signal"
        ),
    )
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_thresholds_are_stable() {
        assert_eq!(tier_for_score(39.9), "Not Eligible");
        assert_eq!(tier_for_score(40.0), "Burd Basic");
        assert_eq!(tier_for_score(60.0), "Burd Plus");
        assert_eq!(tier_for_score(75.0), "Burd Pro");
        assert_eq!(tier_for_score(90.0), "Burd Max");
        assert_eq!(tier_for_score(97.0), "Burd Enterprise");
    }

    #[test]
    fn score_report_serializes() {
        let system = SystemReport {
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
            cpu: "cpu".to_string(),
            cpu_cores: 16,
            ram_total_gb: 64.0,
            ram_available_gb: 48.0,
            gpus: vec![],
            gpu_count: 1,
            primary_gpu_name: Some("NVIDIA GeForce RTX 4090".to_string()),
            vram_per_gpu_gb: Some(24.0),
            vram_total_gb: Some(24.0),
            backend_detected: "CUDA".to_string(),
            cuda_available: true,
            rocm_available: false,
            nvidia_driver: Some("555".to_string()),
            amd_driver: None,
            container_detected: false,
            vm_detected: false,
            timestamp: "2026-06-08T00:00:00Z".to_string(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: "test".to_string(),
        };
        let report = calculate_score(&system, None, None, None, None, None);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("prices_are_demonstrative"));
    }
}
