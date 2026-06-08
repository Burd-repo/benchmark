use crate::pricing::PricingReport;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarningsReport {
    pub daily_estimated_brl: f64,
    pub monthly_estimated_brl: f64,
    pub total_earned_brl_future: f64,
    pub daily_earned_brl_future: f64,
    pub active_jobs_future: u32,
    pub total_jobs_future: u32,
    pub utilization_assumption_pct: f64,
    pub note: String,
    pub warning: String,
}

pub fn estimate_earnings(pricing: &PricingReport) -> EarningsReport {
    let utilization = 0.60;
    let daily = pricing.final_suggested_price_brl_hour * 24.0 * utilization;
    EarningsReport {
        daily_estimated_brl: round2(daily),
        monthly_estimated_brl: round2(daily * 30.0),
        total_earned_brl_future: 0.0,
        daily_earned_brl_future: 0.0,
        active_jobs_future: 0,
        total_jobs_future: 0,
        utilization_assumption_pct: utilization * 100.0,
        note: "Demonstrative only; no marketplace payouts are implemented in this MVP.".to_string(),
        warning: "Ganhos sao estimativas demonstrativas e dependem de demanda, disponibilidade, performance, reputacao e regras da plataforma.".to_string(),
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earnings_uses_demonstrative_warning() {
        let pricing = PricingReport {
            cpu_price_brl_hour: 0.1,
            memory_price_brl_gb_hour: 0.01,
            storage_price_brl_gb_hour: 0.001,
            gpu_price_brl_hour: 5.0,
            endpoint_price_brl_hour_future: 0.0,
            ip_price_brl_hour_future: 0.0,
            final_suggested_price_brl_hour: 5.0,
            prices_are_demonstrative: true,
            warnings: vec![],
        };
        let earnings = estimate_earnings(&pricing);
        assert!(earnings.monthly_estimated_brl > earnings.daily_estimated_brl);
        assert!(earnings.warning.contains("estimativas demonstrativas"));
    }
}
