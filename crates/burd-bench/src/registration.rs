use crate::provider::build_provider_details;
use crate::report::load_latest_signed_report;
use burd_hardware::{BENCHMARK_VERSION, MarketplaceGpuPolicy};
use burd_protocol::{AgentConfig, SignedReport, load_identity};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRegistrationPayload {
    pub provider_id: String,
    pub machine_id: String,
    pub public_key: Option<String>,
    pub agent_version: String,
    pub benchmark_version: String,
    pub hardware_fingerprint: String,
    pub marketplace_policy: MarketplaceGpuPolicy,
    pub provider_details: serde_json::Value,
    pub latest_signed_report_hash: Option<String>,
    pub latest_score: Option<f64>,
    pub latest_tier: Option<String>,
    pub location: serde_json::Value,
    pub contact: serde_json::Value,
    pub capabilities: serde_json::Value,
    pub pricing: serde_json::Value,
    pub verification: serde_json::Value,
    pub evidence: serde_json::Value,
    pub created_at: String,
    pub secrets_included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRegistrationExport {
    pub output: String,
    pub payload: ProviderRegistrationPayload,
}

pub fn build_registration_payload(agent_version: &str) -> ProviderRegistrationPayload {
    let provider = build_provider_details(agent_version, "http://127.0.0.1:8787");
    let identity = load_identity().ok();
    let latest_signed = load_latest_signed_report().ok();
    build_registration_payload_from(
        agent_version,
        &provider,
        identity.as_ref(),
        latest_signed.as_ref(),
        &provider.verification,
        Utc::now().to_rfc3339(),
    )
}

pub(crate) fn build_registration_payload_from(
    agent_version: &str,
    provider: &crate::provider::BurdProviderDetails,
    identity: Option<&AgentConfig>,
    latest_signed: Option<&SignedReport>,
    verification: &crate::verification::ProviderVerification,
    created_at: String,
) -> ProviderRegistrationPayload {
    let latest_score = latest_signed
        .filter(|_| verification.signed_report_current)
        .and_then(|report| report.report.score.get("burd_compute_score"))
        .and_then(|value| value.as_f64())
        .or(Some(provider.score.burd_compute_score));
    let latest_tier = latest_signed
        .filter(|_| verification.signed_report_current)
        .and_then(|report| report.report.score.get("tier"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or(Some(provider.tier.clone()));
    let mut capabilities = serde_json::json!({
        "gpu_count": provider.hardware.gpu_count,
        "vram_gb": provider.hardware.vram_gb,
        "backend": provider.hardware.backend.clone(),
        "hardware_fingerprint": provider.hardware_fingerprint.clone(),
        "marketplace_policy": provider.marketplace_policy.clone(),
        "recommended_workloads": provider.score.recommended_workloads.clone(),
        "container_orchestration_future": false,
        "marketplace_jobs_future": false,
    });
    if let Some(map) = capabilities.as_object_mut() {
        if let Some(source) = provider.hardware.vram_source.clone() {
            map.insert("vram_source".to_string(), serde_json::json!(source));
        }
        if let Some(confidence) = provider.hardware.vram_confidence.clone() {
            map.insert("vram_confidence".to_string(), serde_json::json!(confidence));
        }
    }

    ProviderRegistrationPayload {
        provider_id: provider.provider_id.clone(),
        machine_id: provider.machine_id.clone(),
        public_key: provider.public_key.clone(),
        agent_version: agent_version.to_string(),
        benchmark_version: BENCHMARK_VERSION.to_string(),
        hardware_fingerprint: provider.hardware_fingerprint.clone(),
        marketplace_policy: provider.marketplace_policy.clone(),
        provider_details: serde_json::json!({
            "host_uri": provider.host_uri.clone(),
            "created_at": provider.created_at.clone(),
            "hardware_fingerprint": provider.hardware_fingerprint.clone(),
            "marketplace_policy": provider.marketplace_policy.clone(),
            "hardware": provider.hardware.clone(),
            "gpu_models": provider.gpu_models.clone(),
            "attributes": provider.attributes.clone(),
            "tier": provider.tier.clone(),
            "score": provider.score.burd_compute_score,
        }),
        latest_signed_report_hash: latest_signed.map(|report| report.report_hash.clone()),
        latest_score,
        latest_tier,
        location: serde_json::json!({
            "country": identity.as_ref().and_then(|config| config.country.clone()),
            "region": identity.as_ref().and_then(|config| config.region.clone()),
            "city": identity.as_ref().and_then(|config| config.city.clone()),
        }),
        contact: serde_json::json!({
            "email": identity.as_ref().and_then(|config| config.email.clone()),
            "website": identity.as_ref().and_then(|config| config.website.clone()),
        }),
        capabilities,
        pricing: serde_json::to_value(&provider.pricing)
            .unwrap_or_else(|_| serde_json::json!({"error": "pricing serialization failed"})),
        verification: serde_json::to_value(verification)
            .unwrap_or_else(|_| serde_json::json!({"error": "verification serialization failed"})),
        evidence: serde_json::json!({
            "signed_report": verification.signed_report_evidence.clone(),
            "challenge": verification.challenge_evidence.clone(),
        }),
        created_at,
        secrets_included: false,
    }
}

pub fn export_registration_payload(
    agent_version: &str,
    output: &Path,
) -> Result<ProviderRegistrationExport, String> {
    let payload = build_registration_payload(agent_version);
    if let Some(dir) = output.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("failed to serialize registration payload: {error}"))?;
    fs::write(output, json)
        .map_err(|error| format!("failed to write {}: {error}", output.display()))?;
    Ok(ProviderRegistrationExport {
        output: output.display().to_string(),
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_payload_marks_secrets_absent() {
        let payload = ProviderRegistrationPayload {
            provider_id: "provider".to_string(),
            machine_id: "machine".to_string(),
            public_key: Some("public".to_string()),
            agent_version: "0.1.0".to_string(),
            benchmark_version: "test".to_string(),
            hardware_fingerprint: "sha256:test".to_string(),
            marketplace_policy: burd_hardware::MarketplaceGpuPolicy {
                marketplace_eligible: false,
                eligibility_level: "not_eligible".to_string(),
                gpu_policy: burd_hardware::MARKETPLACE_GPU_POLICY.to_string(),
                requires_nvidia: true,
                requires_cuda: true,
                requires_detected_vram: true,
                minimum_class: "rtx_30xx_or_datacenter".to_string(),
                reasons: vec![],
            },
            provider_details: serde_json::json!({}),
            latest_signed_report_hash: None,
            latest_score: Some(80.0),
            latest_tier: Some("Burd Pro".to_string()),
            location: serde_json::json!({}),
            contact: serde_json::json!({}),
            capabilities: serde_json::json!({}),
            pricing: serde_json::json!({}),
            verification: serde_json::json!({}),
            evidence: serde_json::json!({}),
            created_at: "2026-06-08T00:00:00Z".to_string(),
            secrets_included: false,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(!json.contains("private_key"));
        assert!(!json.contains("api_token"));
    }

    #[test]
    fn registration_payload_preserves_vram_source_and_confidence() {
        let mut provider = crate::test_fixtures::provider_details();
        provider.hardware.vram_source = Some("vulkan_device_memory".to_string());
        provider.hardware.vram_confidence = Some("detected".to_string());
        let verification = crate::test_fixtures::provider_verification();

        let payload = build_registration_payload_from(
            "0.1.0",
            &provider,
            None,
            None,
            &verification,
            "2026-06-08T00:00:00Z".to_string(),
        );

        assert_eq!(payload.capabilities["vram_source"], "vulkan_device_memory");
        assert_eq!(payload.capabilities["vram_confidence"], "detected");
        assert_eq!(
            payload.provider_details["hardware"]["vram_source"],
            "vulkan_device_memory"
        );
    }

    #[test]
    fn registration_payload_contains_fingerprint_and_marketplace_policy() {
        let provider = crate::test_fixtures::provider_details();
        let verification = crate::test_fixtures::provider_verification();

        let payload = build_registration_payload_from(
            "0.1.0",
            &provider,
            None,
            None,
            &verification,
            "2026-06-08T00:00:00Z".to_string(),
        );

        assert_eq!(payload.hardware_fingerprint, provider.hardware_fingerprint);
        assert!(payload.marketplace_policy.marketplace_eligible);
        assert_eq!(
            payload.provider_details["hardware_fingerprint"],
            provider.hardware_fingerprint
        );
        assert_eq!(
            payload.capabilities["marketplace_policy"]["gpu_policy"],
            burd_hardware::MARKETPLACE_GPU_POLICY
        );
    }
}
