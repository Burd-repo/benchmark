use crate::history::{BenchmarkHistoryList, load_history_list};
use crate::report::{load_latest_signed_report, verify_signed_report_at};
use crate::verification::{ProviderVerification, verify_provider_from_reports};
use burd_hardware::{SystemReport, build_system_report, detect_specs};
use burd_llmfit::{BurdFitModel, FitReport, build_fit_report};
use burd_protocol::{
    SIGNED_REPORT_TTL_SECONDS, SignedReport, evidence_freshness_at, load_identity,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPerformanceReport {
    pub status: String,
    pub level: String,
    pub profile: String,
    pub source: String,
    pub confidence_level: String,
    pub measured_at: Option<String>,
    pub expires_at: Option<String>,
    pub is_expired: bool,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub runtime: Option<String>,
    pub backend: Option<String>,
    pub driver: Option<String>,
    pub cuda_version: Option<String>,
    pub gpu_name: Option<String>,
    pub vram_total_gb: Option<f64>,
    pub tokens_per_second: Option<f64>,
    pub tokens_per_second_source: String,
    pub tokens_per_second_confidence: String,
    pub sustained_tokens_per_second: Option<f64>,
    pub sustained_tokens_per_second_source: String,
    pub sustained_tokens_per_second_confidence: String,
    pub time_to_first_token_ms: Option<f64>,
    pub time_to_first_token_ms_source: String,
    pub time_to_first_token_ms_confidence: String,
    pub requests_per_second: Option<f64>,
    pub latency_p50_ms: Option<f64>,
    pub latency_p95_ms: Option<f64>,
    pub stability_passed: Option<bool>,
    pub benchmark_runs: Option<usize>,
    pub benchmark_profile: Option<String>,
    pub compatible_models: Vec<String>,
    pub limited_models: Vec<String>,
    pub max_recommended_model_class: Option<String>,
    pub components: serde_json::Value,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AiPerformanceInputs<'a> {
    pub system: &'a SystemReport,
    pub fit: &'a FitReport,
    pub verification: Option<&'a ProviderVerification>,
    pub latest_signed: Option<&'a SignedReport>,
    pub current_llm_benchmark: Option<&'a serde_json::Value>,
    pub current_measured_at: Option<&'a str>,
    pub history: Option<&'a BenchmarkHistoryList>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct MeasuredCandidate {
    source: &'static str,
    confidence: &'static str,
    measured_at: Option<String>,
    expires_at: Option<String>,
    is_expired: bool,
    llm: serde_json::Value,
    warnings: Vec<String>,
}

pub fn build_ai_performance_report(agent_version: &str) -> AiPerformanceReport {
    let specs = detect_specs();
    let system = build_system_report(&specs, agent_version);
    let fit = build_fit_report(&specs, Some(25));
    let score = crate::score::calculate_score(&system, Some(&fit), None, None, None, None);
    let latest_signed_result = load_latest_signed_report();
    let latest_signed = latest_signed_result.as_ref().ok();
    let verification = verify_provider_from_reports(
        load_identity().as_ref().map(|_| ()).map_err(Clone::clone),
        &system,
        &score,
        latest_signed_result.clone(),
        burd_protocol::load_latest_challenge_output(),
    );
    let history = load_history_list().ok();
    calculate_ai_performance_report(AiPerformanceInputs {
        system: &system,
        fit: &fit,
        verification: Some(&verification),
        latest_signed,
        current_llm_benchmark: None,
        current_measured_at: None,
        history: history.as_ref(),
        now: Utc::now(),
    })
}

pub fn calculate_ai_performance_report(inputs: AiPerformanceInputs<'_>) -> AiPerformanceReport {
    if let Some(candidate) =
        current_candidate(inputs.current_llm_benchmark, inputs.current_measured_at)
            .or_else(|| signed_candidate(inputs.latest_signed, inputs.now))
            .or_else(|| history_candidate(inputs.history, inputs.now))
    {
        return report_from_measured(inputs, candidate);
    }

    report_from_fit_estimate(inputs)
}

fn current_candidate(
    current_llm: Option<&serde_json::Value>,
    measured_at: Option<&str>,
) -> Option<MeasuredCandidate> {
    let llm = current_llm?;
    if is_skipped_or_missing(llm) {
        return None;
    }
    Some(MeasuredCandidate {
        source: "real_benchmark",
        confidence: "high",
        measured_at: measured_at.map(ToOwned::to_owned),
        expires_at: None,
        is_expired: false,
        llm: llm.clone(),
        warnings: Vec::new(),
    })
}
fn signed_candidate(
    latest_signed: Option<&SignedReport>,
    now: DateTime<Utc>,
) -> Option<MeasuredCandidate> {
    let signed = latest_signed?;
    let llm = signed.report.llm_benchmark.as_ref()?;
    if is_skipped_or_missing(llm) {
        return None;
    }
    let verification = verify_signed_report_at(signed, now);
    let evidence = verification.evidence;
    let is_expired = evidence.as_ref().is_some_and(|item| item.is_expired);
    let mut warnings = Vec::new();
    if is_expired {
        warnings.push("latest signed AI benchmark evidence is expired".to_string());
    }
    if !verification.signature_valid {
        warnings.push(
            "latest signed report signature is invalid; AI performance evidence is not trusted"
                .to_string(),
        );
    }
    if !llm
        .get("passed")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        warnings.push("latest signed LLM benchmark did not pass".to_string());
    }

    Some(MeasuredCandidate {
        source: "signed_report",
        confidence: if verification.signature_valid && !is_expired {
            "high"
        } else {
            "medium"
        },
        measured_at: Some(signed.report.timestamp.clone()),
        expires_at: evidence.as_ref().map(|item| item.expires_at.clone()),
        is_expired,
        llm: llm.clone(),
        warnings,
    })
}

fn history_candidate(
    history: Option<&BenchmarkHistoryList>,
    now: DateTime<Utc>,
) -> Option<MeasuredCandidate> {
    let entry = history?.entries.iter().rev().find(|entry| {
        !is_skipped_or_missing(&entry.llm_benchmark_summary)
            && entry
                .llm_benchmark_summary
                .get("avg_tps")
                .and_then(|value| value.as_f64())
                .is_some_and(|value| value > 0.0)
    })?;
    let evidence = evidence_freshness_at(&entry.timestamp, SIGNED_REPORT_TTL_SECONDS, now).ok();
    let is_expired = evidence.as_ref().is_some_and(|item| item.is_expired);
    let mut warnings = Vec::new();
    if is_expired {
        warnings.push("latest benchmark history AI evidence is expired".to_string());
    }
    Some(MeasuredCandidate {
        source: "benchmark_history",
        confidence: if is_expired { "low" } else { "medium" },
        measured_at: Some(entry.timestamp.clone()),
        expires_at: evidence.as_ref().map(|item| item.expires_at.clone()),
        is_expired,
        llm: entry.llm_benchmark_summary.clone(),
        warnings,
    })
}

fn report_from_measured(
    inputs: AiPerformanceInputs<'_>,
    candidate: MeasuredCandidate,
) -> AiPerformanceReport {
    let avg_tps = json_f64(&candidate.llm, &["avg_tps", "tokens_per_second"]);
    let min_tps = json_f64(&candidate.llm, &["min_tps", "sustained_tokens_per_second"]);
    let avg_ttft = json_f64(&candidate.llm, &["avg_ttft_ms", "time_to_first_token_ms"]);
    let latency = json_f64(&candidate.llm, &["avg_latency_ms", "latency_ms"]);
    let run_latencies = run_latencies(&candidate.llm);
    let latency_p50 = percentile(&run_latencies, 0.50).or(latency);
    let latency_p95 = percentile(&run_latencies, 0.95);
    let runs = candidate
        .llm
        .get("runs")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize);
    let passed = candidate
        .llm
        .get("passed")
        .and_then(|value| value.as_bool());
    let status = if candidate.is_expired {
        "expired"
    } else if passed == Some(false) {
        "partial"
    } else {
        "measured"
    };
    let mut warnings = candidate.warnings;
    warnings.extend(json_string_array(&candidate.llm, "warnings"));
    if avg_ttft.is_none() {
        warnings
            .push("time-to-first-token was not measured by the available benchmark".to_string());
    }
    if latency_p95.is_none() {
        warnings.push("latency p95 was not measured by the available benchmark".to_string());
    }

    AiPerformanceReport {
        status: status.to_string(),
        level: level_for_tps(avg_tps),
        profile: "llm_inference".to_string(),
        source: candidate.source.to_string(),
        confidence_level: candidate.confidence.to_string(),
        measured_at: candidate.measured_at,
        expires_at: candidate.expires_at,
        is_expired: candidate.is_expired,
        model: json_string(&candidate.llm, &["model"]),
        provider: json_string(&candidate.llm, &["provider"]),
        runtime: json_string(&candidate.llm, &["runtime", "provider"]),
        backend: Some(inputs.system.backend_detected.clone()),
        driver: inputs
            .system
            .nvidia_driver
            .clone()
            .or_else(|| inputs.system.amd_driver.clone()),
        cuda_version: None,
        gpu_name: inputs.system.primary_gpu_name.clone(),
        vram_total_gb: inputs
            .system
            .vram_total_gb
            .or(inputs.system.vram_per_gpu_gb),
        tokens_per_second: avg_tps,
        tokens_per_second_source: candidate.source.to_string(),
        tokens_per_second_confidence: candidate.confidence.to_string(),
        sustained_tokens_per_second: min_tps,
        sustained_tokens_per_second_source: if min_tps.is_some() {
            candidate.source
        } else {
            "not_measured"
        }
        .to_string(),
        sustained_tokens_per_second_confidence: if min_tps.is_some() {
            candidate.confidence
        } else {
            "unavailable"
        }
        .to_string(),
        time_to_first_token_ms: avg_ttft,
        time_to_first_token_ms_source: if avg_ttft.is_some() {
            candidate.source
        } else {
            "not_measured"
        }
        .to_string(),
        time_to_first_token_ms_confidence: if avg_ttft.is_some() {
            candidate.confidence
        } else {
            "unavailable"
        }
        .to_string(),
        requests_per_second: None,
        latency_p50_ms: latency_p50,
        latency_p95_ms: latency_p95,
        stability_passed: stability_from_warnings(&warnings, passed),
        benchmark_runs: runs,
        benchmark_profile: Some(profile_for_system(inputs.system)),
        compatible_models: compatible_models(inputs.fit),
        limited_models: limited_models(inputs.fit),
        max_recommended_model_class: max_model_class(inputs.system),
        components: serde_json::json!({
            "fit_source": inputs.fit.source,
            "signed_report_current": inputs.verification.map(|value| value.signed_report_current),
            "llm_benchmark_passed": passed,
            "raw_metric_keys": candidate.llm.as_object().map(|map| map.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),
        }),
        warnings: dedupe(warnings),
        notes: base_notes(),
    }
}

fn report_from_fit_estimate(inputs: AiPerformanceInputs<'_>) -> AiPerformanceReport {
    let top = top_fit_model(inputs.fit);
    let estimated_tps = top
        .map(|model| model.estimated_tps)
        .filter(|value| *value > 0.0);
    let has_fit = top.is_some();
    let mut warnings = Vec::new();
    if has_fit {
        warnings.push(
            "no real AI benchmark evidence is available; tokens_per_second is a fit estimate"
                .to_string(),
        );
    } else {
        warnings.push("no measured AI benchmark or runnable fit estimate is available".to_string());
    }

    AiPerformanceReport {
        status: if has_fit { "estimated" } else { "not_measured" }.to_string(),
        level: if has_fit {
            level_for_tps(estimated_tps)
        } else {
            "unknown".to_string()
        },
        profile: "llm_inference".to_string(),
        source: if has_fit {
            "fit_estimate"
        } else {
            "not_measured"
        }
        .to_string(),
        confidence_level: if has_fit { "low" } else { "unavailable" }.to_string(),
        measured_at: None,
        expires_at: None,
        is_expired: false,
        model: top.map(|model| model.name.clone()),
        provider: top.map(|model| model.provider.clone()),
        runtime: top.map(|model| model.runtime.clone()),
        backend: Some(inputs.system.backend_detected.clone()),
        driver: inputs
            .system
            .nvidia_driver
            .clone()
            .or_else(|| inputs.system.amd_driver.clone()),
        cuda_version: None,
        gpu_name: inputs.system.primary_gpu_name.clone(),
        vram_total_gb: inputs
            .system
            .vram_total_gb
            .or(inputs.system.vram_per_gpu_gb),
        tokens_per_second: estimated_tps,
        tokens_per_second_source: if has_fit {
            "fit_estimate"
        } else {
            "not_measured"
        }
        .to_string(),
        tokens_per_second_confidence: if has_fit { "low" } else { "unavailable" }.to_string(),
        sustained_tokens_per_second: None,
        sustained_tokens_per_second_source: "not_measured".to_string(),
        sustained_tokens_per_second_confidence: "unavailable".to_string(),
        time_to_first_token_ms: None,
        time_to_first_token_ms_source: "not_measured".to_string(),
        time_to_first_token_ms_confidence: "unavailable".to_string(),
        requests_per_second: None,
        latency_p50_ms: None,
        latency_p95_ms: None,
        stability_passed: None,
        benchmark_runs: None,
        benchmark_profile: Some(profile_for_system(inputs.system)),
        compatible_models: compatible_models(inputs.fit),
        limited_models: limited_models(inputs.fit),
        max_recommended_model_class: max_model_class(inputs.system),
        components: serde_json::json!({
            "fit_source": inputs.fit.source,
            "runnable_models": inputs.fit.runnable_models,
            "estimated": has_fit,
        }),
        warnings,
        notes: base_notes(),
    }
}

fn top_fit_model(fit: &FitReport) -> Option<&BurdFitModel> {
    fit.models
        .iter()
        .find(|model| model.fit_level != "Too Tight")
}

fn compatible_models(fit: &FitReport) -> Vec<String> {
    fit.models
        .iter()
        .filter(|model| matches!(model.fit_level.as_str(), "Perfect" | "Good"))
        .map(|model| model.name.clone())
        .collect()
}

fn limited_models(fit: &FitReport) -> Vec<String> {
    fit.models
        .iter()
        .filter(|model| model.fit_level == "Marginal")
        .map(|model| model.name.clone())
        .collect()
}

fn max_model_class(system: &SystemReport) -> Option<String> {
    let vram = system.vram_total_gb.or(system.vram_per_gpu_gb)?;
    Some(
        if vram >= 80.0 {
            "very_large"
        } else if vram >= 48.0 {
            "large"
        } else if vram >= 24.0 {
            "medium"
        } else if vram >= 8.0 {
            "small"
        } else {
            "tiny"
        }
        .to_string(),
    )
}

fn profile_for_system(system: &SystemReport) -> String {
    let vram = system
        .vram_total_gb
        .or(system.vram_per_gpu_gb)
        .unwrap_or(0.0);
    crate::profiles::profile_for_vram(vram).id
}

fn level_for_tps(tps: Option<f64>) -> String {
    let Some(tps) = tps else {
        return "unknown".to_string();
    };
    if tps >= 120.0 {
        "enterprise"
    } else if tps >= 70.0 {
        "high"
    } else if tps >= 30.0 {
        "good"
    } else if tps >= 10.0 {
        "basic"
    } else {
        "limited"
    }
    .to_string()
}

fn stability_from_warnings(warnings: &[String], passed: Option<bool>) -> Option<bool> {
    passed.map(|value| {
        value
            && !warnings
                .iter()
                .any(|warning| warning.to_ascii_lowercase().contains("variance"))
    })
}

fn is_skipped_or_missing(value: &serde_json::Value) -> bool {
    value
        .get("status")
        .and_then(|item| item.as_str())
        .is_some_and(|status| matches!(status, "skipped" | "missing" | "not_measured" | "failed"))
}

fn json_f64(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_f64()))
}

fn json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|item| item.as_str())
            .map(ToOwned::to_owned)
    })
}

fn json_string_array(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn run_latencies(value: &serde_json::Value) -> Vec<f64> {
    value
        .get("run_details")
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("latency_ms").and_then(|value| value.as_f64()))
                .collect()
        })
        .unwrap_or_default()
}

fn percentile(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let index = ((sorted.len() - 1) as f64 * percentile).ceil() as usize;
    Some(round1(sorted[index]))
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn dedupe(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn base_notes() -> Vec<String> {
    vec![
        "AI performance metrics consolidate local signed reports, benchmark history, capability spot evidence, and fit estimates.".to_string(),
        "This report does not execute benchmarks automatically and does not start an external runtime.".to_string(),
        "Fit estimates are never treated as measured benchmark proof.".to_string(),
        "Local evidence does not imply backend approval, remote Proof of Capability, scheduler admission, or marketplace eligibility.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 9, 0, 0, 0).unwrap()
    }

    #[test]
    fn no_benchmark_and_no_fit_returns_not_measured() {
        let mut fit = crate::test_fixtures::fit_report();
        fit.models.clear();
        fit.runnable_models = 0;
        let report = calculate_ai_performance_report(AiPerformanceInputs {
            system: &crate::test_fixtures::system_report(),
            fit: &fit,
            verification: Some(&crate::test_fixtures::provider_verification()),
            latest_signed: None,
            current_llm_benchmark: None,
            current_measured_at: None,
            history: None,
            now: now(),
        });

        assert_eq!(report.status, "not_measured");
        assert_eq!(report.tokens_per_second_source, "not_measured");
        assert!(report.time_to_first_token_ms.is_none());
    }

    #[test]
    fn fit_estimate_returns_estimated_never_measured() {
        let report = calculate_ai_performance_report(AiPerformanceInputs {
            system: &crate::test_fixtures::system_report(),
            fit: &crate::test_fixtures::fit_report(),
            verification: Some(&crate::test_fixtures::provider_verification()),
            latest_signed: None,
            current_llm_benchmark: None,
            current_measured_at: None,
            history: None,
            now: now(),
        });

        assert_eq!(report.status, "estimated");
        assert_eq!(report.source, "fit_estimate");
        assert_eq!(report.tokens_per_second_source, "fit_estimate");
        assert_eq!(report.tokens_per_second_confidence, "low");
    }

    #[test]
    fn signed_real_benchmark_returns_measured_with_sources() {
        let signed = crate::test_fixtures::signed_report_with_llm_benchmark(true);
        let report = calculate_ai_performance_report(AiPerformanceInputs {
            system: &crate::test_fixtures::system_report(),
            fit: &crate::test_fixtures::fit_report(),
            verification: Some(&crate::test_fixtures::provider_verification()),
            latest_signed: Some(&signed),
            current_llm_benchmark: None,
            current_measured_at: None,
            history: Some(&crate::test_fixtures::history_list()),
            now: now(),
        });

        assert_eq!(report.status, "measured");
        assert_eq!(report.source, "signed_report");
        assert_eq!(report.confidence_level, "medium");
        assert_eq!(report.tokens_per_second, Some(42.0));
        assert_eq!(report.tokens_per_second_source, "signed_report");
        assert_eq!(report.time_to_first_token_ms, Some(180.0));
        assert_eq!(report.latency_p50_ms, Some(840.0));
        assert_eq!(report.latency_p95_ms, Some(870.0));
    }

    #[test]
    fn signed_expired_benchmark_reduces_confidence() {
        let signed = crate::test_fixtures::signed_report_with_llm_benchmark(true);
        let expired_now = Utc.with_ymd_and_hms(2026, 6, 16, 0, 0, 0).unwrap();
        let report = calculate_ai_performance_report(AiPerformanceInputs {
            system: &crate::test_fixtures::system_report(),
            fit: &crate::test_fixtures::fit_report(),
            verification: Some(&crate::test_fixtures::provider_verification()),
            latest_signed: Some(&signed),
            current_llm_benchmark: None,
            current_measured_at: None,
            history: None,
            now: expired_now,
        });

        assert_eq!(report.status, "expired");
        assert!(report.is_expired);
        assert_eq!(report.confidence_level, "medium");
        assert!(report.warnings.iter().any(|item| item.contains("expired")));
    }

    #[test]
    fn missing_metrics_stay_null_and_not_measured() {
        let mut signed = crate::test_fixtures::signed_report_with_llm_benchmark(true);
        signed.report.llm_benchmark = Some(serde_json::json!({
            "provider": "ollama",
            "model": "fixture-8b",
            "runtime": "ollama",
            "runs": 1,
            "avg_tps": 42.0,
            "passed": true,
            "warnings": [],
            "errors": []
        }));
        let report = calculate_ai_performance_report(AiPerformanceInputs {
            system: &crate::test_fixtures::system_report(),
            fit: &crate::test_fixtures::fit_report(),
            verification: Some(&crate::test_fixtures::provider_verification()),
            latest_signed: Some(&signed),
            current_llm_benchmark: None,
            current_measured_at: None,
            history: None,
            now: now(),
        });

        assert_eq!(report.status, "measured");
        assert!(report.time_to_first_token_ms.is_none());
        assert_eq!(report.time_to_first_token_ms_source, "not_measured");
        assert!(report.requests_per_second.is_none());
        assert!(report.latency_p95_ms.is_none());
    }
}
