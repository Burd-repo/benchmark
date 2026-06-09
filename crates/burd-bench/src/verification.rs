use crate::report::{load_latest_signed_report, verify_signed_report};
use crate::score::calculate_score;
use burd_hardware::{build_system_report, detect_specs};
use burd_llmfit::build_fit_report;
use burd_protocol::load_identity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderVerification {
    pub hardware_verified: bool,
    pub benchmark_verified: bool,
    pub signature_verified: bool,
    pub challenge_verified: bool,
    pub uptime_verified: bool,
    pub network_verified: bool,
    pub disk_verified: bool,
    pub llm_runtime_verified: bool,
    pub fraud_risk_level: String,
    pub audit_status: String,
    pub warnings: Vec<String>,
    pub failed_checks: Vec<String>,
}

pub fn verify_provider(agent_version: &str) -> ProviderVerification {
    let specs = detect_specs();
    let system = build_system_report(&specs, agent_version);
    let fit = build_fit_report(&specs, Some(25));
    let score = calculate_score(&system, Some(&fit), None, None, None, None);
    verify_provider_from_reports(
        load_identity().map(|_| ()),
        &system,
        &score,
        load_latest_signed_report(),
    )
}

pub(crate) fn verify_provider_from_reports(
    identity: Result<(), String>,
    system: &burd_hardware::SystemReport,
    score: &crate::score::ScoreReport,
    signed_result: Result<burd_protocol::SignedReport, String>,
) -> ProviderVerification {
    let mut warnings = Vec::new();
    let mut failed_checks = Vec::new();

    let identity_ok = match identity {
        Ok(()) => true,
        Err(error) => {
            failed_checks.push("identity_missing".to_string());
            warnings.push(error);
            false
        }
    };

    let hardware_verified = system.cpu_cores > 0 && !system.cpu.trim().is_empty();
    if !hardware_verified {
        failed_checks.push("hardware_not_detected".to_string());
    }

    if system.gpu_count > 0 && system.vram_total_gb.or(system.vram_per_gpu_gb).is_none() {
        warnings.push("GPU detected but VRAM was not confirmed by hardware detection".to_string());
    }
    if system.backend_detected.to_lowercase().contains("cpu")
        && score.suggested_price_brl_hour > 2.0
    {
        warnings.push("CPU fallback with elevated suggested price".to_string());
    }
    if score.burd_compute_score < 40.0 && score.eligible {
        failed_checks.push("score_tier_inconsistent".to_string());
    }

    let signed = signed_result.as_ref().ok();
    let signature_verified = match signed {
        Some(report) => {
            let verification = verify_signed_report(report);
            if !verification.signature_valid {
                failed_checks.push("report_signature_invalid".to_string());
            }
            verification.signature_valid && verification.errors.is_empty()
        }
        None => {
            let error = signed_result
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_else(|| "unknown signed report error".to_string());
            warnings.push(format!("signed report unavailable: {error}"));
            failed_checks.push("signed_report_missing".to_string());
            false
        }
    };

    let latest_score = signed
        .as_ref()
        .and_then(|report| report.report.score.get("burd_compute_score"))
        .and_then(|value| value.as_f64())
        .unwrap_or(score.burd_compute_score);
    let benchmark_verified = latest_score >= 40.0;
    if !benchmark_verified {
        warnings.push("Burd Compute Score is below marketplace readiness threshold".to_string());
    }

    let llm_runtime_verified = signature_verified && benchmark_verified;
    let network_verified = signature_verified;
    let disk_verified = signature_verified;
    let uptime_verified = false;
    let challenge_verified = false;

    let fraud_risk_level =
        if failed_checks.iter().any(|item| item.contains("signature")) || !identity_ok {
            "high"
        } else if !benchmark_verified || !warnings.is_empty() {
            "medium"
        } else {
            "low"
        };

    let audit_status = if signature_verified && hardware_verified && benchmark_verified {
        "self_verified"
    } else {
        "not_audited"
    };

    ProviderVerification {
        hardware_verified,
        benchmark_verified,
        signature_verified,
        challenge_verified,
        uptime_verified,
        network_verified,
        disk_verified,
        llm_runtime_verified,
        fraud_risk_level: fraud_risk_level.to_string(),
        audit_status: audit_status.to_string(),
        warnings,
        failed_checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_serializes() {
        let value = ProviderVerification {
            hardware_verified: true,
            benchmark_verified: false,
            signature_verified: false,
            challenge_verified: false,
            uptime_verified: false,
            network_verified: false,
            disk_verified: false,
            llm_runtime_verified: false,
            fraud_risk_level: "unknown".to_string(),
            audit_status: "not_audited".to_string(),
            warnings: vec!["warning".to_string()],
            failed_checks: vec!["signature".to_string()],
        };
        let json = serde_json::to_string(&value).unwrap();
        assert!(json.contains("fraud_risk_level"));
    }
}
