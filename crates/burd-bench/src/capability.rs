use crate::history::{BenchmarkHistoryList, load_history_list};
use crate::report::load_latest_signed_report;
use crate::verification::{ProviderVerification, verify_provider};
use burd_hardware::{SystemReport, build_system_report, detect_specs};
use burd_llmfit::{FitReport, build_fit_report};
use burd_protocol::SignedReport;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySpotVerificationReport {
    pub capability_score: f64,
    pub level: String,
    pub status: String,
    pub verification_mode: String,
    pub checked_at: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_model: Option<String>,
    pub runnable_models: usize,
    pub recommended_workloads: Vec<String>,
    pub components: CapabilitySpotComponents,
    pub checks: Vec<CapabilitySpotCheck>,
    pub evidence: CapabilitySpotEvidence,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySpotComponents {
    pub fit_evidence: f64,
    pub runtime_readiness: f64,
    pub benchmark_evidence: f64,
    pub verification_integrity: f64,
    pub history_support: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySpotCheck {
    pub id: String,
    pub label: String,
    pub status: String,
    pub score: f64,
    pub max_score: f64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySpotEvidence {
    pub signed_report_current: bool,
    pub challenge_verified: bool,
    pub llm_benchmark_current: bool,
    pub llm_benchmark_passed: bool,
    pub history_entries: usize,
}

pub fn build_capability_spot_verification(
    agent_version: &str,
) -> CapabilitySpotVerificationReport {
    let latest_signed = load_latest_signed_report().ok();
    let history = load_history_list().ok();
    build_capability_spot_verification_from(agent_version, latest_signed.as_ref(), history.as_ref())
}

pub fn build_capability_spot_verification_from(
    agent_version: &str,
    latest_signed: Option<&SignedReport>,
    history: Option<&BenchmarkHistoryList>,
) -> CapabilitySpotVerificationReport {
    let specs = detect_specs();
    let system = build_system_report(&specs, agent_version);
    let fit = build_fit_report(&specs, Some(25));
    let verification = verify_provider(agent_version);
    calculate_capability_spot_verification(&system, &fit, &verification, latest_signed, history)
}

pub fn calculate_capability_spot_verification(
    system: &SystemReport,
    fit: &FitReport,
    verification: &ProviderVerification,
    latest_signed: Option<&SignedReport>,
    history: Option<&BenchmarkHistoryList>,
) -> CapabilitySpotVerificationReport {
    let fit_evidence = fit_evidence_score(fit);
    let runtime_readiness = runtime_readiness_score(system, verification);
    let benchmark = llm_benchmark_evidence(latest_signed, verification.signed_report_current);
    let verification_integrity = verification_integrity_score(verification);
    let history_support = history_support_score(history);

    let capability_score = round1(
        fit_evidence * 0.30
            + runtime_readiness * 0.20
            + benchmark.score * 0.20
            + verification_integrity * 0.20
            + history_support * 0.10,
    );

    let mut warnings = Vec::new();
    if !verification.signed_report_current {
        warnings.push("signed report evidence is not current for capability spot verification".to_string());
    }
    if !benchmark.current {
        warnings.push("no current live llm benchmark evidence is attached to the latest signed report".to_string());
    } else if !benchmark.passed {
        warnings.push("latest signed report includes a failed llm benchmark".to_string());
    }
    if !verification.challenge_verified {
        warnings.push("challenge-backed capability evidence is not available yet".to_string());
    }
    if fit.runnable_models == 0 {
        warnings.push("fit analysis did not find runnable models for the current hardware snapshot".to_string());
    }

    let checks = vec![
        check(
            "fit_evidence",
            "Model fit evidence",
            fit_evidence,
            fit_message(fit),
        ),
        check(
            "runtime_readiness",
            "Runtime readiness",
            runtime_readiness,
            runtime_message(system, verification),
        ),
        check(
            "benchmark_evidence",
            "Live benchmark evidence",
            benchmark.score,
            benchmark.message.clone(),
        ),
        check(
            "verification_integrity",
            "Verification integrity",
            verification_integrity,
            verification_message(verification),
        ),
        check(
            "history_support",
            "History support",
            history_support,
            history_message(history),
        ),
    ];

    let top_model = fit
        .models
        .iter()
        .find(|model| model.fit_level != "Too Tight")
        .map(|model| model.name.clone());
    let checked_at = latest_signed
        .map(|report| report.signed_at.clone())
        .unwrap_or_else(|| system.timestamp.clone());
    let history_entries = history.map(|value| value.entries_total).unwrap_or(0);

    CapabilitySpotVerificationReport {
        capability_score,
        level: capability_level(capability_score).to_string(),
        status: capability_status(capability_score, verification, &benchmark).to_string(),
        verification_mode: "local_mock".to_string(),
        checked_at,
        summary: capability_summary(
            capability_score,
            fit,
            verification,
            benchmark.current,
            benchmark.passed,
        ),
        top_model,
        runnable_models: fit.runnable_models,
        recommended_workloads: fit.recommended_workloads.clone(),
        components: CapabilitySpotComponents {
            fit_evidence: round1(fit_evidence),
            runtime_readiness: round1(runtime_readiness),
            benchmark_evidence: round1(benchmark.score),
            verification_integrity: round1(verification_integrity),
            history_support: round1(history_support),
        },
        checks,
        evidence: CapabilitySpotEvidence {
            signed_report_current: verification.signed_report_current,
            challenge_verified: verification.challenge_verified,
            llm_benchmark_current: benchmark.current,
            llm_benchmark_passed: benchmark.passed,
            history_entries,
        },
        warnings,
        notes: vec![
            "Capability spot verification is a local/mock signal derived from fit analysis, runtime state, signed evidence, and optional live benchmark proof.".to_string(),
            "It does not create workload eligibility, marketplace admission, backend approval, or a scheduling guarantee.".to_string(),
            "A current signed report with a passing llm benchmark is stronger evidence than fit-only capability inference.".to_string(),
        ],
    }
}

#[derive(Debug, Clone)]
struct BenchmarkEvidence {
    current: bool,
    passed: bool,
    score: f64,
    message: String,
}

fn fit_evidence_score(fit: &FitReport) -> f64 {
    if fit.runnable_models == 0 {
        return 0.0;
    }
    if fit.runnable_models >= 10 {
        return 100.0;
    }
    55.0 + (fit.runnable_models.min(9) as f64 * 5.0)
}

fn runtime_readiness_score(system: &SystemReport, verification: &ProviderVerification) -> f64 {
    let mut score: f64 = 0.0;
    if system.gpu_count > 0 {
        score += 35.0;
    }
    if system.backend_detected != "CPU" {
        score += 25.0;
    }
    if verification.llm_runtime_verified {
        score += 20.0;
    }
    if system.vram_total_gb.or(system.vram_per_gpu_gb).unwrap_or(0.0) >= 8.0 {
        score += 20.0;
    }
    score.min(100.0)
}

fn llm_benchmark_evidence(
    latest_signed: Option<&SignedReport>,
    signed_report_current: bool,
) -> BenchmarkEvidence {
    let Some(report) = latest_signed else {
        return BenchmarkEvidence {
            current: false,
            passed: false,
            score: 35.0,
            message: "no signed report with llm benchmark evidence is available".to_string(),
        };
    };
    let Some(llm) = report.report.llm_benchmark.as_ref() else {
        return BenchmarkEvidence {
            current: false,
            passed: false,
            score: 35.0,
            message: "latest signed report does not include llm benchmark evidence".to_string(),
        };
    };
    if llm.get("status").and_then(|value| value.as_str()) == Some("skipped") {
        return BenchmarkEvidence {
            current: false,
            passed: false,
            score: 40.0,
            message: "latest signed report skipped live llm benchmark execution".to_string(),
        };
    }

    let passed = llm
        .get("passed")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let avg_tps = llm
        .get("avg_tps")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    let current = signed_report_current;

    let score = if !current {
        45.0
    } else if passed && avg_tps >= 20.0 {
        100.0
    } else if passed && avg_tps > 0.0 {
        85.0
    } else {
        20.0
    };

    let message = if !current {
        "latest signed report contains llm benchmark evidence, but the signed report is stale".to_string()
    } else if passed {
        format!("latest signed report includes a passing llm benchmark (avg_tps {:.1})", avg_tps)
    } else {
        "latest signed report includes llm benchmark evidence, but it did not pass".to_string()
    };

    BenchmarkEvidence {
        current,
        passed,
        score,
        message,
    }
}

fn verification_integrity_score(verification: &ProviderVerification) -> f64 {
    let mut score: f64 = 0.0;
    if verification.hardware_verified {
        score += 20.0;
    }
    if verification.signature_verified {
        score += 20.0;
    }
    if verification.fingerprint_matches {
        score += 20.0;
    }
    if verification.signed_report_current {
        score += 15.0;
    }
    if verification.challenge_verified {
        score += 15.0;
    }
    if verification.audit_status == "self_verified" {
        score += 10.0;
    }
    match verification.fraud_risk_level.as_str() {
        "high" => (score - 50.0).max(0.0),
        "medium" => (score - 20.0).max(0.0),
        _ => score.min(100.0),
    }
}

fn history_support_score(history: Option<&BenchmarkHistoryList>) -> f64 {
    let entries = history.map(|value| value.entries_total).unwrap_or(0);
    match entries {
        0 => 25.0,
        1 => 55.0,
        2 => 75.0,
        _ => 100.0,
    }
}

fn fit_message(fit: &FitReport) -> String {
    if fit.runnable_models == 0 {
        "fit analysis found no runnable models".to_string()
    } else {
        format!(
            "fit analysis found {} runnable models and {} recommended workloads",
            fit.runnable_models,
            fit.recommended_workloads.len()
        )
    }
}

fn runtime_message(system: &SystemReport, verification: &ProviderVerification) -> String {
    format!(
        "runtime uses {} with {} gpu(s); llm runtime verified: {}",
        system.backend_detected,
        system.gpu_count,
        verification.llm_runtime_verified
    )
}

fn verification_message(verification: &ProviderVerification) -> String {
    format!(
        "signed current: {}, fingerprint matches: {}, fraud risk: {}",
        verification.signed_report_current,
        verification.fingerprint_matches,
        verification.fraud_risk_level
    )
}

fn history_message(history: Option<&BenchmarkHistoryList>) -> String {
    let entries = history.map(|value| value.entries_total).unwrap_or(0);
    format!("local benchmark history contains {entries} entrie(s)")
}

fn capability_status(
    capability_score: f64,
    verification: &ProviderVerification,
    benchmark: &BenchmarkEvidence,
) -> &'static str {
    if verification.fraud_risk_level == "high" || !verification.fingerprint_matches {
        "failed"
    } else if benchmark.current && !benchmark.passed {
        "degraded"
    } else if capability_score >= 80.0 {
        "verified_locally"
    } else if capability_score >= 55.0 {
        "partially_verified"
    } else {
        "insufficient_evidence"
    }
}

fn capability_level(score: f64) -> &'static str {
    if score >= 90.0 {
        "Strong"
    } else if score >= 75.0 {
        "Good"
    } else if score >= 55.0 {
        "Limited"
    } else {
        "Weak"
    }
}

fn capability_summary(
    capability_score: f64,
    fit: &FitReport,
    verification: &ProviderVerification,
    benchmark_current: bool,
    benchmark_passed: bool,
) -> String {
    let fit_state = if fit.runnable_models > 0 {
        "fit evidence supports local AI capability"
    } else {
        "fit evidence does not currently support local AI capability"
    };
    let benchmark_state = if benchmark_current && benchmark_passed {
        "live benchmark evidence is current"
    } else if benchmark_current {
        "live benchmark evidence is present but degraded"
    } else {
        "live benchmark evidence is not current"
    };
    format!(
        "{fit_state}; {benchmark_state}; capability score {:.1}; verification status {}.",
        capability_score, verification.audit_status
    )
}

fn check(id: &str, label: &str, score: f64, message: String) -> CapabilitySpotCheck {
    CapabilitySpotCheck {
        id: id.to_string(),
        label: label.to_string(),
        status: check_status(score).to_string(),
        score: round1(score),
        max_score: 100.0,
        message,
    }
}

fn check_status(score: f64) -> &'static str {
    if score >= 80.0 {
        "passed"
    } else if score >= 50.0 {
        "partial"
    } else {
        "failed"
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_spot_verification_is_strong_with_passing_llm_benchmark() {
        let report = calculate_capability_spot_verification(
            &crate::test_fixtures::system_report(),
            &crate::test_fixtures::fit_report(),
            &crate::test_fixtures::provider_verification(),
            Some(&crate::test_fixtures::signed_report_with_llm_benchmark(true)),
            Some(&crate::test_fixtures::history_list()),
        );

        assert!(report.capability_score >= 80.0);
        assert_eq!(report.status, "verified_locally");
        assert!(report.evidence.llm_benchmark_current);
        assert!(report.evidence.llm_benchmark_passed);
    }

    #[test]
    fn capability_spot_verification_degrades_without_current_benchmark() {
        let report = calculate_capability_spot_verification(
            &crate::test_fixtures::system_report(),
            &crate::test_fixtures::fit_report(),
            &crate::test_fixtures::provider_verification(),
            Some(&crate::test_fixtures::synthetic_signed_report(crate::test_fixtures::full_report(None))),
            Some(&crate::test_fixtures::history_list()),
        );

        assert!(!report.evidence.llm_benchmark_current);
        assert!(report.warnings.iter().any(|warning| warning.contains("no current live llm benchmark evidence")));
        assert!(report.capability_score < 90.0);
    }
}




