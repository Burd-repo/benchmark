use crate::report::{load_latest_signed_report, verify_signed_report_at};
use crate::score::calculate_score;
use burd_hardware::{build_system_report, detect_specs, hardware_fingerprint};
use burd_llmfit::build_fit_report;
use burd_protocol::{
    ChallengeRunOutput, EvidenceFreshness, load_identity, load_latest_challenge_output,
    verify_challenge_response,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderVerification {
    pub hardware_verified: bool,
    pub hardware_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_report_hardware_fingerprint: Option<String>,
    pub fingerprint_matches: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_report_evidence: Option<EvidenceFreshness>,
    pub signed_report_current: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_evidence: Option<EvidenceFreshness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_confidence: Option<String>,
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
        load_latest_challenge_output(),
    )
}

pub(crate) fn verify_provider_from_reports(
    identity: Result<(), String>,
    system: &burd_hardware::SystemReport,
    score: &crate::score::ScoreReport,
    signed_result: Result<burd_protocol::SignedReport, String>,
    challenge_result: Result<ChallengeRunOutput, String>,
) -> ProviderVerification {
    verify_provider_from_reports_at(
        identity,
        system,
        score,
        signed_result,
        challenge_result,
        Utc::now(),
    )
}

pub(crate) fn verify_provider_from_reports_at(
    identity: Result<(), String>,
    system: &burd_hardware::SystemReport,
    score: &crate::score::ScoreReport,
    signed_result: Result<burd_protocol::SignedReport, String>,
    challenge_result: Result<ChallengeRunOutput, String>,
    now: DateTime<Utc>,
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
    let current_hardware_fingerprint = hardware_fingerprint(system);
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
    let signed_verification = signed.map(|report| verify_signed_report_at(report, now));
    let signature_verified = match signed_verification.as_ref() {
        Some(verification) => {
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
    let signed_report_evidence = signed_verification
        .as_ref()
        .and_then(|verification| verification.evidence.clone());
    let signed_report_current = signed_report_evidence
        .as_ref()
        .is_some_and(|evidence| !evidence.is_expired);
    if signature_verified && !signed_report_current {
        warnings.push("Latest signed report is expired".to_string());
        failed_checks.push("signed_report_expired".to_string());
    }
    let signed_report_hardware_fingerprint =
        signed.and_then(|report| report.report.hardware_fingerprint.clone());
    let fingerprint_matches = match signed_report_hardware_fingerprint.as_deref() {
        Some(fingerprint) if fingerprint == current_hardware_fingerprint => true,
        Some(_) => {
            warnings.push(
                "Current hardware fingerprint differs from the latest signed report".to_string(),
            );
            failed_checks.push("hardware_fingerprint_mismatch".to_string());
            false
        }
        None => {
            warnings.push("Latest signed report does not include hardware fingerprint".to_string());
            failed_checks.push("report_hardware_fingerprint_missing".to_string());
            false
        }
    };

    let latest_score = signed
        .as_ref()
        .and_then(|report| report.report.score.get("burd_compute_score"))
        .and_then(|value| value.as_f64())
        .unwrap_or(score.burd_compute_score);
    let benchmark_verified = latest_score >= 40.0 && signed_report_current;
    if !benchmark_verified {
        warnings.push("Burd Compute Score is below marketplace readiness threshold".to_string());
    }

    let llm_runtime_verified = signature_verified && benchmark_verified;
    let network_verified = signature_verified && signed_report_current;
    let disk_verified = signature_verified && signed_report_current;
    let uptime_verified = false;
    let challenge_verification = challenge_result
        .as_ref()
        .ok()
        .map(|output| verify_challenge_response(&output.challenge, &output.response));
    let challenge_evidence = challenge_verification
        .as_ref()
        .and_then(|verification| verification.evidence.clone());
    if challenge_evidence
        .as_ref()
        .is_some_and(|evidence| evidence.is_expired)
    {
        warnings.push("Latest challenge evidence is expired".to_string());
        failed_checks.push("challenge_expired".to_string());
    }
    let challenge_verified = challenge_result.as_ref().ok().is_some_and(|output| {
        let verification = challenge_verification
            .as_ref()
            .expect("challenge verification exists for challenge output");
        verification.valid
            && verification.signature_valid
            && !verification.expired
            && output.response.hardware_fingerprint.as_deref()
                == Some(current_hardware_fingerprint.as_str())
    });
    if let Ok(output) = challenge_result.as_ref() {
        match output.response.hardware_fingerprint.as_deref() {
            Some(fingerprint) if fingerprint != current_hardware_fingerprint => {
                warnings.push(
                    "Current hardware fingerprint differs from the latest challenge response"
                        .to_string(),
                );
                failed_checks.push("challenge_hardware_fingerprint_mismatch".to_string());
            }
            None => {
                warnings.push(
                    "Latest challenge response does not include hardware fingerprint".to_string(),
                );
                failed_checks.push("challenge_hardware_fingerprint_missing".to_string());
            }
            _ => {}
        }
    }

    let fraud_risk_level = if failed_checks.iter().any(|item| {
        item.contains("signature")
            || item == "hardware_fingerprint_mismatch"
            || item == "challenge_hardware_fingerprint_mismatch"
    }) || !identity_ok
    {
        "high"
    } else if !benchmark_verified || !warnings.is_empty() {
        "medium"
    } else {
        "low"
    };

    let audit_status =
        if signature_verified && hardware_verified && benchmark_verified && fingerprint_matches {
            "self_verified"
        } else {
            "not_audited"
        };

    ProviderVerification {
        hardware_verified,
        hardware_fingerprint: current_hardware_fingerprint,
        signed_report_hardware_fingerprint,
        fingerprint_matches,
        signed_report_evidence,
        signed_report_current,
        challenge_evidence,
        vram_source: system.vram_source.clone(),
        vram_confidence: system.vram_confidence.clone(),
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
    use chrono::DateTime;

    #[test]
    fn verification_serializes() {
        let value = ProviderVerification {
            hardware_verified: true,
            hardware_fingerprint: "sha256:current".to_string(),
            signed_report_hardware_fingerprint: Some("sha256:current".to_string()),
            fingerprint_matches: true,
            signed_report_evidence: None,
            signed_report_current: true,
            challenge_evidence: None,
            vram_source: Some("vulkan_device_memory".to_string()),
            vram_confidence: Some("detected".to_string()),
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

    #[test]
    fn detected_vram_does_not_add_provider_verification_warning() {
        let mut system = crate::test_fixtures::system_report();
        system.vram_source = Some("vulkan_device_memory".to_string());
        system.vram_confidence = Some("detected".to_string());
        let now = DateTime::parse_from_rfc3339("2026-06-09T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let verification = verify_provider_from_reports_at(
            Ok(()),
            &system,
            &crate::test_fixtures::score_report(),
            crate::test_fixtures::signed_report(None),
            Err("challenge unavailable".to_string()),
            now,
        );

        assert!(verification.hardware_verified);
        assert_eq!(
            verification.vram_source.as_deref(),
            Some("vulkan_device_memory")
        );
        assert_eq!(verification.vram_confidence.as_deref(), Some("detected"));
        assert!(
            !verification
                .warnings
                .iter()
                .any(|warning| warning.contains("VRAM"))
        );
    }
}
