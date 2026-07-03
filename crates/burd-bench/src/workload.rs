use crate::capability::{CapabilitySpotVerificationReport, calculate_capability_spot_verification};
use crate::health::{ReliabilityReport, calculate_reliability, load_reliability_report};
use crate::history::{BenchmarkHistoryList, load_history_list};
use crate::network::{calculate_network_score, load_network_score_report};
use crate::report::load_latest_signed_report;
use crate::score::{ScoreReport, calculate_score};
use crate::trust::{TrustScoreReport, calculate_trust_score};
use crate::verification::{ProviderVerification, verify_provider};
use burd_hardware::{MarketplaceGpuPolicy, SystemReport, build_system_report, detect_specs};
use burd_llmfit::{FitReport, build_fit_report};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadEligibilityReport {
    pub checked_at: String,
    pub provider_tier: String,
    pub local_status: String,
    pub marketplace_status_future: String,
    pub workloads: Vec<WorkloadEligibility>,
    pub summary: String,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadEligibility {
    pub workload: String,
    pub local_status: String,
    pub marketplace_status_future: String,
    pub confidence_level: String,
    pub capability_score: f64,
    pub trust_score: f64,
    pub summary: String,
    pub reasons: Vec<String>,
    pub blockers: Vec<String>,
}

pub fn build_workload_eligibility(agent_version: &str) -> WorkloadEligibilityReport {
    let specs = detect_specs();
    let system = build_system_report(&specs, agent_version);
    let fit = build_fit_report(&specs, Some(25));
    let score = calculate_score(&system, Some(&fit), None, None, None, None);
    let verification = verify_provider(agent_version);
    let reliability = load_reliability_report().unwrap_or_else(|_| calculate_reliability(&[]));
    let network = load_network_score_report().unwrap_or_else(|_| calculate_network_score(None));
    let history = load_history_list().ok();
    let latest_signed = load_latest_signed_report().ok();
    let capability = calculate_capability_spot_verification(
        &system,
        &fit,
        &verification,
        latest_signed.as_ref(),
        history.as_ref(),
    );
    let trust = calculate_trust_score(
        &verification,
        &reliability,
        &network,
        history.as_ref(),
        None,
    );
    calculate_workload_eligibility(
        &system,
        &fit,
        &score,
        &verification,
        &reliability,
        &capability,
        &trust,
        history.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn calculate_workload_eligibility(
    system: &SystemReport,
    fit: &FitReport,
    score: &ScoreReport,
    verification: &ProviderVerification,
    reliability: &ReliabilityReport,
    capability: &CapabilitySpotVerificationReport,
    trust: &TrustScoreReport,
    history: Option<&BenchmarkHistoryList>,
) -> WorkloadEligibilityReport {
    let workloads = canonical_workloads(fit);
    let items: Vec<WorkloadEligibility> = workloads
        .into_iter()
        .map(|workload| {
            evaluate_workload(
                &workload,
                &system,
                fit,
                score,
                verification,
                reliability,
                capability,
                trust,
            )
        })
        .collect();

    let local_status = if items
        .iter()
        .any(|item| item.local_status == "eligible_locally")
    {
        "eligible_locally"
    } else if items
        .iter()
        .any(|item| item.local_status == "diagnostic_only")
    {
        "diagnostic_only"
    } else {
        "not_ready"
    };
    let marketplace_status_future = if items
        .iter()
        .any(|item| item.marketplace_status_future == "marketplace_candidate")
    {
        "marketplace_candidate"
    } else {
        "marketplace_blocked"
    };

    let mut warnings = Vec::new();
    if !verification.signed_report_current {
        warnings.push(
            "signed report evidence is not current; workload eligibility remains conservative"
                .to_string(),
        );
    }
    if capability.status != "verified_locally" {
        warnings.push("capability spot verification is below verified_locally; some workloads remain diagnostic-only".to_string());
    }
    if !system.cuda_available && system.backend_detected != "CUDA" {
        warnings.push("non-cuda or cpu fallback runtimes keep marketplace workload eligibility blocked in the local MVP".to_string());
    }
    if history.map(|value| value.entries_total).unwrap_or(0) == 0 {
        warnings.push("no benchmark history is available; workload confidence is based on current local signals only".to_string());
    }

    WorkloadEligibilityReport {
        checked_at: system.timestamp.clone(),
        provider_tier: score.tier.clone(),
        local_status: local_status.to_string(),
        marketplace_status_future: marketplace_status_future.to_string(),
        workloads: items,
        summary: format!(
            "Local workload eligibility is {} and future marketplace workload eligibility is {}; capability {:.1}; trust {:.1}.",
            local_status, marketplace_status_future, capability.capability_score, trust.trust_score
        ),
        warnings,
        notes: vec![
            "Workload eligibility is a local decision layer built on top of fit analysis, capability spot verification, trust score, and marketplace GPU policy.".to_string(),
            "Local eligibility does not create a backend lease, a scheduler assignment, or a paid marketplace admission.".to_string(),
            "Future marketplace eligibility remains stricter than local eligibility and is blocked when marketplace policy or evidence quality is insufficient.".to_string(),
        ],
    }
}

fn canonical_workloads(fit: &FitReport) -> Vec<String> {
    let mut workloads = vec![
        "LLM inference".to_string(),
        "chatbots".to_string(),
        "coding agents".to_string(),
        "reasoning agents".to_string(),
        "embeddings".to_string(),
        "batch inference pequeno".to_string(),
        "SDXL".to_string(),
        "fine-tuning".to_string(),
    ];
    for workload in fit
        .recommended_workloads
        .iter()
        .chain(fit.not_recommended_workloads.iter())
    {
        if !workloads
            .iter()
            .any(|item| normalize(item) == normalize(workload))
        {
            workloads.push(workload.clone());
        }
    }
    workloads
}

#[allow(clippy::too_many_arguments)]
fn evaluate_workload(
    workload: &str,
    system: &SystemReport,
    fit: &FitReport,
    score: &ScoreReport,
    verification: &ProviderVerification,
    reliability: &ReliabilityReport,
    capability: &CapabilitySpotVerificationReport,
    trust: &TrustScoreReport,
) -> WorkloadEligibility {
    let normalized = normalize(workload);
    let recommended = fit
        .recommended_workloads
        .iter()
        .any(|item| normalize(item) == normalized)
        || fit.models.iter().any(|model| {
            model
                .workloads
                .iter()
                .any(|item| normalize(item) == normalized)
        });
    let explicitly_not_recommended = fit
        .not_recommended_workloads
        .iter()
        .any(|item| normalize(item) == normalized);

    let mut reasons = Vec::new();
    let mut blockers = Vec::new();

    if recommended {
        reasons.push(
            "fit analysis recommends this workload for the current hardware snapshot".to_string(),
        );
    } else if explicitly_not_recommended {
        blockers.push("fit analysis explicitly marks this workload as not recommended".to_string());
    } else {
        reasons.push("workload is not explicitly recommended, so eligibility falls back to generic local capability signals".to_string());
    }

    if verification.signature_verified
        && verification.hardware_verified
        && verification.fingerprint_matches
    {
        reasons.push(
            "provider verification keeps the current hardware and signature evidence coherent"
                .to_string(),
        );
    } else {
        blockers.push(
            "provider verification is missing current hardware or signature integrity".to_string(),
        );
    }

    if capability.evidence.llm_benchmark_current && capability.evidence.llm_benchmark_passed {
        reasons.push("current signed evidence includes a passing local llm benchmark".to_string());
    }

    let local_status = local_workload_status(
        recommended,
        explicitly_not_recommended,
        capability.capability_score,
        trust.trust_score,
        verification,
        &mut blockers,
    );
    let marketplace_status_future = if explicitly_not_recommended {
        blockers.push("this workload is explicitly outside the current local recommendation set for future marketplace use".to_string());
        "marketplace_blocked"
    } else {
        marketplace_workload_status(
            workload,
            &system,
            &score,
            reliability,
            capability,
            trust,
            verification,
            &mut blockers,
        )
    };
    let confidence_level =
        confidence_level(capability.capability_score, trust.trust_score).to_string();

    WorkloadEligibility {
        workload: workload.to_string(),
        local_status: local_status.to_string(),
        marketplace_status_future: marketplace_status_future.to_string(),
        confidence_level,
        capability_score: capability.capability_score,
        trust_score: trust.trust_score,
        summary: format!(
            "{} locally; {} for future marketplace policy.",
            local_status, marketplace_status_future
        ),
        reasons,
        blockers,
    }
}

fn local_workload_status(
    recommended: bool,
    explicitly_not_recommended: bool,
    capability_score: f64,
    trust_score: f64,
    verification: &ProviderVerification,
    blockers: &mut Vec<String>,
) -> &'static str {
    if verification.fraud_risk_level == "high" || !verification.fingerprint_matches {
        if !blockers.iter().any(|item| item.contains("fraud")) {
            blockers.push(
                "fraud risk or fingerprint mismatch blocks local workload eligibility".to_string(),
            );
        }
        return "blocked";
    }
    if explicitly_not_recommended {
        return "not_recommended";
    }
    if recommended && capability_score >= 70.0 && trust_score >= 60.0 {
        return "eligible_locally";
    }
    if capability_score >= 55.0 && trust_score >= 45.0 {
        return "diagnostic_only";
    }
    blockers.push("capability or trust is too weak for local workload eligibility".to_string());
    "blocked"
}

#[allow(clippy::too_many_arguments)]
fn marketplace_workload_status(
    workload: &str,
    system: &SystemReport,
    score: &ScoreReport,
    reliability: &ReliabilityReport,
    capability: &CapabilitySpotVerificationReport,
    trust: &TrustScoreReport,
    verification: &ProviderVerification,
    blockers: &mut Vec<String>,
) -> &'static str {
    let policy = marketplace_policy(system);
    if !policy.marketplace_eligible {
        blockers.push(
            "marketplace gpu policy does not allow this machine into the paid marketplace path"
                .to_string(),
        );
        return "marketplace_blocked";
    }
    if capability.status != "verified_locally" {
        blockers.push(
            "capability spot verification is not strong enough for future marketplace eligibility"
                .to_string(),
        );
        return "marketplace_blocked";
    }
    if trust.trust_score < 70.0 {
        blockers.push("trust score is below the local future-marketplace threshold".to_string());
        return "marketplace_blocked";
    }
    if reliability.reliability_score < 70.0 {
        blockers
            .push("reliability score is below the local future-marketplace threshold".to_string());
        return "marketplace_blocked";
    }
    if !verification.signed_report_current || !verification.signature_verified {
        blockers.push(
            "signed report evidence is not current and verified for future marketplace eligibility"
                .to_string(),
        );
        return "marketplace_blocked";
    }
    if score.burd_compute_score < workload_min_compute_score(workload) {
        blockers.push("compute score is below the local threshold for this workload".to_string());
        return "marketplace_blocked";
    }
    "marketplace_candidate"
}

fn workload_min_compute_score(workload: &str) -> f64 {
    match normalize(workload).as_str() {
        "fine tuning" | "fine-tuning" => 90.0,
        "sdxl" => 75.0,
        "coding agents" | "reasoning agents" => 65.0,
        "batch inference pequeno" | "batch inference" => 60.0,
        _ => 50.0,
    }
}

fn confidence_level(capability_score: f64, trust_score: f64) -> &'static str {
    let blended = capability_score * 0.6 + trust_score * 0.4;
    if blended >= 85.0 {
        "high"
    } else if blended >= 65.0 {
        "medium"
    } else {
        "low"
    }
}

fn marketplace_policy(system: &SystemReport) -> MarketplaceGpuPolicy {
    burd_hardware::evaluate_marketplace_gpu_policy(system)
}

fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .replace('/', " ")
        .replace('-', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_eligibility_promotes_recommended_local_workloads() {
        let report = calculate_workload_eligibility(
            &crate::test_fixtures::system_report(),
            &crate::test_fixtures::fit_report(),
            &crate::test_fixtures::score_report(),
            &crate::test_fixtures::provider_verification(),
            &crate::test_fixtures::reliability_report(),
            &crate::test_fixtures::capability_spot_report(),
            &crate::test_fixtures::trust_score_report(),
            Some(&crate::test_fixtures::history_list()),
        );

        assert!(
            report
                .workloads
                .iter()
                .any(|item| item.workload == "agentes" && item.local_status == "eligible_locally")
        );
        assert!(
            report
                .workloads
                .iter()
                .any(|item| item.marketplace_status_future == "marketplace_candidate")
        );
    }

    #[test]
    fn workload_eligibility_blocks_heavy_unrecommended_workloads() {
        let report = calculate_workload_eligibility(
            &crate::test_fixtures::system_report(),
            &crate::test_fixtures::fit_report(),
            &crate::test_fixtures::score_report(),
            &crate::test_fixtures::provider_verification(),
            &crate::test_fixtures::reliability_report(),
            &crate::test_fixtures::capability_spot_report(),
            &crate::test_fixtures::trust_score_report(),
            Some(&crate::test_fixtures::history_list()),
        );

        let fine_tuning = report
            .workloads
            .iter()
            .find(|item| normalize(&item.workload) == normalize("fine-tuning pesado"))
            .expect("fine-tuning workload is present");
        assert_eq!(fine_tuning.local_status, "not_recommended");
        assert_eq!(fine_tuning.marketplace_status_future, "marketplace_blocked");
    }
}
