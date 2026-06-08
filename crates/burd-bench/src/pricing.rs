use crate::score::ScoreReport;
use burd_hardware::SystemReport;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingReport {
    pub cpu_price_brl_hour: f64,
    pub memory_price_brl_gb_hour: f64,
    pub storage_price_brl_gb_hour: f64,
    pub gpu_price_brl_hour: f64,
    pub endpoint_price_brl_hour_future: f64,
    pub ip_price_brl_hour_future: f64,
    pub final_suggested_price_brl_hour: f64,
    pub prices_are_demonstrative: bool,
    pub warnings: Vec<String>,
}

pub fn calculate_pricing(system: &SystemReport, score: &ScoreReport) -> PricingReport {
    let cpu_price = round2(system.cpu_cores as f64 * 0.015);
    let memory_price = 0.006;
    let storage_price = 0.0004;
    let gpu_price = score.suggested_price_brl_hour.max(0.0);
    PricingReport {
        cpu_price_brl_hour: cpu_price,
        memory_price_brl_gb_hour: memory_price,
        storage_price_brl_gb_hour: storage_price,
        gpu_price_brl_hour: gpu_price,
        endpoint_price_brl_hour_future: 0.0,
        ip_price_brl_hour_future: 0.0,
        final_suggested_price_brl_hour: round2(gpu_price + cpu_price * 0.1),
        prices_are_demonstrative: true,
        warnings: vec![
            "Prices are demonstrative and depend on future Burd marketplace policy.".to_string(),
        ],
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::{ScoreComponents, ScoreReport};

    #[test]
    fn pricing_is_demonstrative() {
        let system = SystemReport {
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
            cpu: "cpu".to_string(),
            cpu_cores: 8,
            ram_total_gb: 32.0,
            ram_available_gb: 16.0,
            gpus: vec![],
            gpu_count: 1,
            primary_gpu_name: Some("gpu".to_string()),
            vram_per_gpu_gb: Some(24.0),
            vram_total_gb: Some(24.0),
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
        let score = ScoreReport {
            burd_compute_score: 75.0,
            tier: "Burd Pro".to_string(),
            eligible: true,
            recommended_workloads: vec![],
            not_recommended_workloads: vec![],
            suggested_price_brl_hour: 5.0,
            price_basis: "test".to_string(),
            prices_are_demonstrative: true,
            components: ScoreComponents {
                llm_benchmark: 0.0,
                vram_capacity: 0.0,
                stability: 0.0,
                network: 0.0,
                disk: 0.0,
                verification: 0.0,
            },
            warnings: vec![],
            notes: vec![],
        };
        let pricing = calculate_pricing(&system, &score);
        assert!(pricing.prices_are_demonstrative);
        assert!(pricing.final_suggested_price_brl_hour >= pricing.gpu_price_brl_hour);
    }
}
