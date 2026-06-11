use crate::disk::{DiskBenchmarkOptions, run_disk_benchmark};
use crate::llm::{LlmBenchmarkOptions, LlmBenchmarkReport, run_llm_benchmark};
use crate::network::{NetworkBenchmarkOptions, run_network_benchmark};
use crate::profiles::profile_for_vram;
use crate::score::calculate_score;
use crate::stability::{StabilityBenchmarkReport, run_stability_benchmark};
use burd_hardware::{
    BENCHMARK_VERSION, SystemReport, build_hardware_fingerprint_report, build_system_report,
    detect_specs,
};
use burd_llmfit::{FitReport, build_fit_report};
use burd_protocol::{
    AgentConfig, Challenge, FULL_REPORT_TTL_SECONDS, FullReport, KEY_ALGORITHM, PrivateKeyFile,
    ReportSignature, SIGNED_REPORT_TTL_SECONDS, SignedReport, VerifyReportResult,
    default_state_dir, evidence_freshness, evidence_freshness_at, hash_canonical, load_identity,
    load_private_key, placeholder_signature, sign_message, verify_message,
};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ReportRunOptions {
    pub run_all: bool,
    pub agent_version: String,
    pub llm_provider: String,
    pub llm_url: Option<String>,
    pub llm_model: Option<String>,
    pub challenge: Option<Challenge>,
}

impl ReportRunOptions {
    pub fn new(agent_version: impl Into<String>) -> Self {
        Self {
            run_all: false,
            agent_version: agent_version.into(),
            llm_provider: "ollama".to_string(),
            llm_url: None,
            llm_model: None,
            challenge: None,
        }
    }
}

pub fn generate_full_report(options: ReportRunOptions) -> FullReport {
    let specs = detect_specs();
    let system = build_system_report(&specs, &options.agent_version);
    let fit = build_fit_report(&specs, Some(25));
    generate_full_report_from_snapshot(options, &system, &fit)
}

pub(crate) fn generate_full_report_from_snapshot(
    options: ReportRunOptions,
    system: &SystemReport,
    fit: &FitReport,
) -> FullReport {
    let vram = system
        .vram_total_gb
        .or(system.vram_per_gpu_gb)
        .unwrap_or(0.0);
    let profile = profile_for_vram(vram);
    let identity = load_identity().ok().map(|config| config.public_identity());

    let mut llm_benchmark: Option<LlmBenchmarkReport> = None;
    let mut stability: Option<StabilityBenchmarkReport> = None;
    let mut network = None;
    let mut disk = None;

    if options.run_all {
        let llm_options = LlmBenchmarkOptions {
            provider: options.llm_provider,
            url: options.llm_url,
            model: options.llm_model,
            runs: profile.default_runs,
            profile: Some(profile.id.clone()),
            detected_vram_gb: vram,
        };
        let llm = run_llm_benchmark(llm_options.clone());
        llm_benchmark = Some(llm);
        stability = Some(run_stability_benchmark(0, llm_options));
        network = Some(run_network_benchmark(NetworkBenchmarkOptions::default()));
        disk = Some(run_disk_benchmark(DiskBenchmarkOptions::default()));
    }

    let score = calculate_score(
        &system,
        Some(fit),
        llm_benchmark.as_ref(),
        stability.as_ref(),
        network.as_ref(),
        disk.as_ref(),
    );

    let machine_id = identity.as_ref().map(|value| value.machine_id.clone());
    let challenge_id = options
        .challenge
        .as_ref()
        .map(|value| value.challenge_id.clone());
    let fingerprint = build_hardware_fingerprint_report(system);
    let timestamp = Utc::now().to_rfc3339();
    FullReport {
        identity,
        evidence: evidence_freshness(&timestamp, FULL_REPORT_TTL_SECONDS).ok(),
        hardware_fingerprint: Some(fingerprint.hardware_fingerprint),
        marketplace_policy: Some(
            serde_json::to_value(fingerprint.marketplace_policy)
                .expect("marketplace policy serializes"),
        ),
        system: serde_json::to_value(system).expect("system report serializes"),
        fit: Some(serde_json::to_value(fit).expect("fit report serializes")),
        llm_benchmark: if let Some(value) = &llm_benchmark {
            Some(serde_json::to_value(value).expect("llm benchmark serializes"))
        } else {
            Some(skipped("not run; use report --run-all or bench llm"))
        },
        stability: if let Some(value) = &stability {
            Some(serde_json::to_value(value).expect("stability report serializes"))
        } else {
            Some(skipped("not run; use report --run-all or bench stability"))
        },
        network: if let Some(value) = &network {
            Some(serde_json::to_value(value).expect("network report serializes"))
        } else {
            Some(skipped("not run; use report --run-all or bench network"))
        },
        disk: if let Some(value) = &disk {
            Some(serde_json::to_value(value).expect("disk report serializes"))
        } else {
            Some(skipped("not run; use report --run-all or bench disk"))
        },
        score: serde_json::to_value(score).expect("score serializes"),
        timestamp,
        agent_version: options.agent_version,
        benchmark_version: BENCHMARK_VERSION.to_string(),
        benchmark_profile: profile.id,
        challenge: options.challenge,
        signature: placeholder_signature(machine_id.as_deref(), challenge_id.as_deref()),
    }
}

pub fn generate_signed_report(options: ReportRunOptions) -> Result<SignedReport, String> {
    let config = load_identity()?;
    let private_key = load_private_key(&config)?;
    let report = generate_full_report(options);
    let signed =
        sign_full_report_with_identity_at(report, config, private_key, Utc::now().to_rfc3339())?;
    let _ = save_latest_signed_report(&signed);
    Ok(signed)
}

#[cfg(test)]
pub(crate) fn sign_full_report_at(
    report: FullReport,
    signed_at: String,
) -> Result<SignedReport, String> {
    let config = load_identity()?;
    let private_key = load_private_key(&config)?;
    sign_full_report_with_identity_at(report, config, private_key, signed_at)
}

fn sign_full_report_with_identity_at(
    mut report: FullReport,
    config: AgentConfig,
    private_key: PrivateKeyFile,
    signed_at: String,
) -> Result<SignedReport, String> {
    report.signature = ReportSignature {
        algorithm: KEY_ALGORITHM.to_string(),
        value: "signature-in-envelope".to_string(),
        status: "signed".to_string(),
    };
    let report_hash = hash_canonical(&report)?;
    let signature = sign_message(&private_key.secret_key_base64, report_hash.as_bytes())?;
    let signature_valid_locally =
        verify_message(&config.public_key, report_hash.as_bytes(), &signature).unwrap_or(false);
    let signed = SignedReport {
        provider_id: config.provider_id,
        machine_id: config.machine_id,
        report,
        report_hash,
        signature,
        public_key: config.public_key,
        key_algorithm: config.key_algorithm,
        evidence: evidence_freshness(&signed_at, SIGNED_REPORT_TTL_SECONDS).ok(),
        signed_at,
        signature_valid_locally,
        canonicalization_version: "burd-json-c14n-v1".to_string(),
    };
    Ok(signed)
}

pub fn verify_signed_report(report: &SignedReport) -> VerifyReportResult {
    verify_signed_report_at(report, Utc::now())
}

pub(crate) fn verify_signed_report_at(
    report: &SignedReport,
    now: DateTime<Utc>,
) -> VerifyReportResult {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    if report.key_algorithm != KEY_ALGORITHM {
        errors.push(format!(
            "unsupported key algorithm '{}'",
            report.key_algorithm
        ));
    }

    let computed_hash = hash_canonical(&report.report).ok();
    if computed_hash.as_deref() != Some(report.report_hash.as_str()) {
        errors.push("report_hash does not match canonical report".to_string());
    }

    let signature_valid = verify_message(
        &report.public_key,
        report.report_hash.as_bytes(),
        &report.signature,
    )
    .unwrap_or_else(|error| {
        errors.push(error);
        false
    });
    if !signature_valid {
        errors.push("signature invalid".to_string());
    }
    if report.report.identity.is_none() {
        warnings.push("report does not include provider identity".to_string());
    }
    if report.report.hardware_fingerprint.is_none() {
        warnings.push("report does not include hardware fingerprint".to_string());
    }
    let evidence = evidence_freshness_at(&report.signed_at, SIGNED_REPORT_TTL_SECONDS, now)
        .map_err(|error| {
            errors.push(error);
        })
        .ok();
    if evidence
        .as_ref()
        .is_some_and(|evidence| evidence.is_expired)
    {
        warnings.push("signed report expired".to_string());
    }

    VerifyReportResult {
        report_hash: computed_hash,
        signature_valid,
        key_algorithm: Some(report.key_algorithm.clone()),
        provider_id: Some(report.provider_id.clone()),
        machine_id: Some(report.machine_id.clone()),
        evidence,
        checked_at: now.to_rfc3339(),
        warnings,
        errors,
    }
}

pub fn save_latest_report(report: &FullReport) -> Result<(), String> {
    let path = default_state_dir().join("latest-report.json");
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialize latest report: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn save_latest_signed_report(report: &SignedReport) -> Result<(), String> {
    let path = default_state_dir().join("latest-signed-report.json");
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialize latest signed report: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn load_latest_signed_report() -> Result<SignedReport, String> {
    let path = default_state_dir().join("latest-signed-report.json");
    load_signed_report_file(&path)
}

pub fn load_signed_report_file(path: &Path) -> Result<SignedReport, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut report: SignedReport = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid signed report JSON: {error}"))?;
    report.evidence = evidence_freshness(&report.signed_at, SIGNED_REPORT_TTL_SECONDS).ok();
    Ok(report)
}

fn skipped(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "skipped",
        "reason": reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_report_shape_is_stable() {
        let value = skipped("reason");
        assert_eq!(value["status"], "skipped");
    }

    #[test]
    fn report_preserves_vram_source_and_confidence_from_system_snapshot() {
        let mut system = crate::test_fixtures::system_report();
        system.vram_source = Some("vulkan_device_memory".to_string());
        system.vram_confidence = Some("detected".to_string());
        system.gpus[0].vram_source = system.vram_source.clone();
        system.gpus[0].vram_confidence = system.vram_confidence.clone();

        let report = generate_full_report_from_snapshot(
            ReportRunOptions::new("0.1.0"),
            &system,
            &crate::test_fixtures::fit_report(),
        );

        assert_eq!(report.system["vram_total_gb"], 24.0);
        assert_eq!(report.system["vram_source"], "vulkan_device_memory");
        assert_eq!(report.system["vram_confidence"], "detected");
        assert_eq!(
            report.system["gpus"][0]["vram_source"],
            "vulkan_device_memory"
        );
    }

    #[test]
    fn signed_report_verification_detects_tamper() {
        let report = SignedReport {
            provider_id: "provider".to_string(),
            machine_id: "machine".to_string(),
            report: FullReport {
                identity: None,
                evidence: None,
                hardware_fingerprint: None,
                marketplace_policy: None,
                system: serde_json::json!({"os": "linux"}),
                fit: None,
                llm_benchmark: None,
                stability: None,
                network: None,
                disk: None,
                score: serde_json::json!({"burd_compute_score": 0}),
                timestamp: "2026-06-08T00:00:00Z".to_string(),
                agent_version: "0.1.0".to_string(),
                benchmark_version: "test".to_string(),
                benchmark_profile: "profile_8gb".to_string(),
                challenge: None,
                signature: ReportSignature {
                    algorithm: KEY_ALGORITHM.to_string(),
                    value: "signature-in-envelope".to_string(),
                    status: "signed".to_string(),
                },
            },
            report_hash: "wrong".to_string(),
            signature: "bad".to_string(),
            public_key: "bad".to_string(),
            key_algorithm: KEY_ALGORITHM.to_string(),
            signed_at: "2026-06-08T00:00:00Z".to_string(),
            evidence: None,
            signature_valid_locally: false,
            canonicalization_version: "burd-json-c14n-v1".to_string(),
        };
        let result = verify_signed_report(&report);
        assert!(!result.signature_valid);
        assert!(!result.errors.is_empty());
    }
}
