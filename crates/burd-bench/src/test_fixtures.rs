use crate::earnings::estimate_earnings;
use crate::health::UptimeSummary;
use crate::pricing::calculate_pricing;
use crate::provider::{
    BurdProviderDetails, GpuModelDetail, ProviderAttribute, ProviderHardware, ProviderLocation,
    ProviderStats, ResourceStat, StorageStats,
};
use crate::report::sign_full_report_at;
use crate::score::{ScoreComponents, ScoreReport};
use crate::verification::ProviderVerification;
use burd_hardware::{
    BENCHMARK_VERSION, GpuReport, SystemReport, build_hardware_fingerprint_report,
};
use burd_llmfit::{BurdFitModel, FitReport};
use burd_protocol::{
    AgentIdentityPublic, Challenge, EvidenceFreshness, FullReport, KEY_ALGORITHM, ReportSignature,
    SignedReport,
};

pub(crate) const FIXTURE_TIMESTAMP: &str = "2026-06-08T00:00:00Z";
pub(crate) const FIXTURE_PROVIDER_ID: &str = "burd-provider-contract";
pub(crate) const FIXTURE_MACHINE_ID: &str = "burd-machine-contract";
pub(crate) const FIXTURE_PUBLIC_KEY: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
pub(crate) const FIXTURE_SECRET_KEY: &str = "nWGxne/9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A=";

pub(crate) fn system_report() -> SystemReport {
    SystemReport {
        os: "linux".to_string(),
        architecture: "x86_64".to_string(),
        cpu: "Burd Contract CPU".to_string(),
        cpu_cores: 16,
        ram_total_gb: 64.0,
        ram_available_gb: 48.0,
        gpus: vec![GpuReport {
            name: "NVIDIA GeForce RTX 4090".to_string(),
            vram_gb: Some(24.0),
            vram_source: Some("nvidia_smi".to_string()),
            vram_confidence: Some("detected".to_string()),
            backend: "CUDA".to_string(),
            count: 1,
            unified_memory: false,
        }],
        gpu_count: 1,
        primary_gpu_name: Some("NVIDIA GeForce RTX 4090".to_string()),
        vram_per_gpu_gb: Some(24.0),
        vram_total_gb: Some(24.0),
        vram_source: Some("nvidia_smi".to_string()),
        vram_confidence: Some("detected".to_string()),
        backend_detected: "CUDA".to_string(),
        cuda_available: true,
        rocm_available: false,
        nvidia_driver: Some("555.42".to_string()),
        amd_driver: None,
        container_detected: false,
        vm_detected: false,
        timestamp: FIXTURE_TIMESTAMP.to_string(),
        agent_version: "0.1.0-test".to_string(),
        benchmark_version: BENCHMARK_VERSION.to_string(),
    }
}

pub(crate) fn fit_report() -> FitReport {
    FitReport {
        models: vec![BurdFitModel {
            name: "Burd Contract Model".to_string(),
            provider: "fixture".to_string(),
            parameter_count: "8B".to_string(),
            fit_level: "Perfect".to_string(),
            run_mode: "GPU".to_string(),
            best_quantization: "Q4_K_M".to_string(),
            memory_estimated_gb: 6.0,
            memory_available_gb: 24.0,
            memory_usage_pct: 25.0,
            estimated_tps: 80.0,
            effective_context: 32_768,
            category: "General".to_string(),
            workloads: vec!["LLM inference".to_string(), "agents".to_string()],
            runtime: "llama.cpp/Ollama".to_string(),
            score: 95.0,
            notes: vec!["deterministic contract fixture".to_string()],
        }],
        recommended_workloads: vec!["LLM medio quantizado".to_string(), "agentes".to_string()],
        not_recommended_workloads: vec!["fine-tuning pesado".to_string()],
        provider_capability_summary: "deterministic CUDA fixture".to_string(),
        total_models_analyzed: 1,
        runnable_models: 1,
        source: "contract fixture".to_string(),
    }
}

pub(crate) fn score_report() -> ScoreReport {
    ScoreReport {
        burd_compute_score: 82.5,
        tier: "Burd Pro".to_string(),
        eligible: true,
        recommended_workloads: fit_report().recommended_workloads,
        not_recommended_workloads: fit_report().not_recommended_workloads,
        suggested_price_brl_hour: 5.9,
        price_basis: "deterministic contract fixture".to_string(),
        prices_are_demonstrative: true,
        components: ScoreComponents {
            llm_benchmark: 75.0,
            vram_capacity: 82.0,
            stability: 90.0,
            network: 80.0,
            disk: 85.0,
            verification: 100.0,
        },
        warnings: vec![],
        notes: vec!["deterministic contract fixture".to_string()],
    }
}

pub(crate) fn full_report(challenge: Option<Challenge>) -> FullReport {
    let fingerprint = build_hardware_fingerprint_report(&system_report());
    FullReport {
        identity: Some(identity()),
        evidence: Some(fixture_evidence()),
        hardware_fingerprint: Some(fingerprint.hardware_fingerprint),
        marketplace_policy: Some(serde_json::to_value(fingerprint.marketplace_policy).unwrap()),
        system: serde_json::to_value(system_report()).unwrap(),
        fit: Some(serde_json::to_value(fit_report()).unwrap()),
        llm_benchmark: Some(skipped("not run in fast contract fixture")),
        stability: Some(skipped("not run in fast contract fixture")),
        network: Some(skipped("not run in fast contract fixture")),
        disk: Some(skipped("not run in fast contract fixture")),
        score: serde_json::to_value(score_report()).unwrap(),
        timestamp: FIXTURE_TIMESTAMP.to_string(),
        agent_version: "0.1.0-test".to_string(),
        benchmark_version: BENCHMARK_VERSION.to_string(),
        benchmark_profile: "profile_24gb".to_string(),
        challenge,
        signature: ReportSignature {
            algorithm: "placeholder-ed25519".to_string(),
            value: "placeholder-signature:burd-machine-contract:no-challenge".to_string(),
            status: "mocked".to_string(),
        },
    }
}

pub(crate) fn signed_report(challenge: Option<Challenge>) -> Result<SignedReport, String> {
    sign_full_report_at(full_report(challenge), FIXTURE_TIMESTAMP.to_string())
}

pub(crate) fn provider_details() -> BurdProviderDetails {
    let system = system_report();
    let score = score_report();
    let pricing = calculate_pricing(&system, &score);
    let uptime = uptime_summary();
    let fingerprint = build_hardware_fingerprint_report(&system);
    BurdProviderDetails {
        provider_id: FIXTURE_PROVIDER_ID.to_string(),
        machine_id: FIXTURE_MACHINE_ID.to_string(),
        public_key: Some(FIXTURE_PUBLIC_KEY.to_string()),
        host_uri: "http://127.0.0.1:8787".to_string(),
        created_at: Some(FIXTURE_TIMESTAMP.to_string()),
        last_check_date: FIXTURE_TIMESTAMP.to_string(),
        is_online: true,
        is_verified: true,
        is_audited: false,
        audit_status: "self_verified".to_string(),
        location: ProviderLocation {
            country: Some("BR".to_string()),
            city: Some("Sao Paulo".to_string()),
            region: Some("br-southeast".to_string()),
            timezone: Some("America/Sao_Paulo".to_string()),
        },
        hardware_fingerprint: fingerprint.hardware_fingerprint,
        marketplace_policy: fingerprint.marketplace_policy,
        hardware: ProviderHardware {
            cpu: system.cpu.clone(),
            architecture: system.architecture.clone(),
            memory_gb: system.ram_total_gb,
            disk_free_gb: Some(512.0),
            backend: system.backend_detected.clone(),
            gpu_count: system.gpu_count,
            vram_gb: system.vram_total_gb,
            vram_source: system.vram_source.clone(),
            vram_confidence: system.vram_confidence.clone(),
        },
        gpu_models: vec![GpuModelDetail {
            vendor: "nvidia".to_string(),
            model: system.primary_gpu_name.clone().unwrap(),
            vram_gb: system.vram_total_gb,
            vram_source: system.vram_source.clone(),
            vram_confidence: system.vram_confidence.clone(),
            count: 1,
        }],
        uptime_1d: uptime.uptime_1d,
        uptime_7d: uptime.uptime_7d,
        uptime_30d: uptime.uptime_30d,
        uptime,
        stats: provider_stats(),
        pricing: pricing.clone(),
        tier: score.tier.clone(),
        score,
        active_jobs_future: 0,
        total_jobs_future: 0,
        estimated_earnings: estimate_earnings(&pricing),
        attributes: vec![
            ProviderAttribute {
                key: "host".to_string(),
                value: "burd".to_string(),
            },
            ProviderAttribute {
                key: "backend".to_string(),
                value: "CUDA".to_string(),
            },
        ],
        logs_summary: serde_json::json!({
            "actions_total": 0,
            "logs_total": 0,
            "latest_action": null,
        }),
        raw_report: serde_json::to_value(full_report(None)).unwrap(),
        verification: provider_verification(),
        backend_verification_status_future: "not_connected".to_string(),
    }
}

pub(crate) fn provider_verification() -> ProviderVerification {
    let hardware_fingerprint =
        build_hardware_fingerprint_report(&system_report()).hardware_fingerprint;
    ProviderVerification {
        hardware_verified: true,
        hardware_fingerprint: hardware_fingerprint.clone(),
        signed_report_hardware_fingerprint: Some(hardware_fingerprint),
        fingerprint_matches: true,
        signed_report_evidence: Some(fixture_evidence()),
        signed_report_current: true,
        challenge_evidence: None,
        vram_source: Some("nvidia_smi".to_string()),
        vram_confidence: Some("detected".to_string()),
        benchmark_verified: true,
        signature_verified: true,
        challenge_verified: false,
        uptime_verified: false,
        network_verified: true,
        disk_verified: true,
        llm_runtime_verified: true,
        fraud_risk_level: "low".to_string(),
        audit_status: "self_verified".to_string(),
        warnings: vec![],
        failed_checks: vec![],
    }
}

pub(crate) fn fixture_evidence() -> EvidenceFreshness {
    EvidenceFreshness {
        issued_at: FIXTURE_TIMESTAMP.to_string(),
        expires_at: "2026-06-15T00:00:00+00:00".to_string(),
        is_expired: false,
        age_seconds: 0,
        ttl_seconds: 604_800,
    }
}

pub(crate) fn identity() -> AgentIdentityPublic {
    AgentIdentityPublic {
        provider_id: FIXTURE_PROVIDER_ID.to_string(),
        machine_id: FIXTURE_MACHINE_ID.to_string(),
        api_url: "https://api.burd.cloud".to_string(),
        preferred_provider: "ollama".to_string(),
        benchmark_profile: "auto".to_string(),
        telemetry_enabled: false,
        created_at: FIXTURE_TIMESTAMP.to_string(),
        public_key: FIXTURE_PUBLIC_KEY.to_string(),
        key_algorithm: KEY_ALGORITHM.to_string(),
        email: None,
        website: None,
        country: Some("BR".to_string()),
        city: Some("Sao Paulo".to_string()),
        region: Some("br-southeast".to_string()),
    }
}

fn uptime_summary() -> UptimeSummary {
    UptimeSummary {
        uptime_1d: 1.0,
        uptime_7d: 1.0,
        uptime_30d: 1.0,
        last_online_at: Some(FIXTURE_TIMESTAMP.to_string()),
        last_failed_check_at: None,
        checks_total: 1,
        checks_failed: 0,
        current_status: "idle".to_string(),
    }
}

fn provider_stats() -> ProviderStats {
    let zero = ResourceStat {
        available: 0.0,
        active: 0.0,
        pending: 0.0,
        total: 0.0,
    };
    ProviderStats {
        cpu: ResourceStat {
            available: 16.0,
            active: 0.0,
            pending: 0.0,
            total: 16.0,
        },
        gpu: ResourceStat {
            available: 1.0,
            active: 0.0,
            pending: 0.0,
            total: 1.0,
        },
        memory: ResourceStat {
            available: 48.0,
            active: 0.0,
            pending: 0.0,
            total: 64.0,
        },
        storage: StorageStats {
            ephemeral: ResourceStat {
                available: 512.0,
                active: 0.0,
                pending: 0.0,
                total: 512.0,
            },
            persistent_future: zero,
            total: ResourceStat {
                available: 512.0,
                active: 0.0,
                pending: 0.0,
                total: 512.0,
            },
        },
    }
}

fn skipped(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "status": "skipped",
        "reason": reason,
    })
}
