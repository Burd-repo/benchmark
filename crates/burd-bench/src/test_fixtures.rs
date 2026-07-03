use crate::capability::CapabilitySpotVerificationReport;
use crate::history::{BenchmarkHistoryEntry, BenchmarkHistoryList, GpuSummary, SystemSummary};
use crate::earnings::estimate_earnings;
use crate::health::{ReliabilityComponents, ReliabilityReport, UptimeSummary};
use crate::network::{NetworkBenchmarkReport, NetworkScoreReport, calculate_network_score};
use crate::pricing::calculate_pricing;
use crate::provider::{
    BurdProviderDetails, GpuModelDetail, ProviderAttribute, ProviderHardware, ProviderLocation,
    ProviderStats, ResourceStat, StorageStats,
};
use crate::report::sign_full_report_at;
use crate::score::{ScoreComponents, ScoreReport};
use crate::trust::TrustScoreReport;
use crate::workload::WorkloadEligibilityReport;
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
        network_score: Some(serde_json::to_value(network_score_report()).unwrap()),
        disk: Some(skipped("not run in fast contract fixture")),
        reliability: Some(serde_json::to_value(reliability_report()).unwrap()),
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

pub(crate) fn synthetic_signed_report(report: FullReport) -> SignedReport {
    SignedReport {
        provider_id: FIXTURE_PROVIDER_ID.to_string(),
        machine_id: FIXTURE_MACHINE_ID.to_string(),
        report,
        report_hash: "sha256:fixture-signed-report".to_string(),
        signature: "fixture-signature".to_string(),
        public_key: FIXTURE_PUBLIC_KEY.to_string(),
        key_algorithm: KEY_ALGORITHM.to_string(),
        signed_at: FIXTURE_TIMESTAMP.to_string(),
        evidence: Some(fixture_evidence()),
        signature_valid_locally: true,
        canonicalization_version: "burd-json-c14n-v1".to_string(),
    }
}

pub(crate) fn signed_report_with_llm_benchmark(passed: bool) -> SignedReport {
    let mut report = full_report(None);
    report.llm_benchmark = Some(serde_json::json!({
        "provider": "ollama",
        "model": "fixture-8b",
        "runtime": "ollama",
        "runs": 3,
        "avg_tps": if passed { 42.0 } else { 0.0 },
        "min_tps": if passed { 39.5 } else { 0.0 },
        "max_tps": if passed { 44.1 } else { 0.0 },
        "stddev_tps": if passed { 1.8 } else { 0.0 },
        "avg_ttft_ms": if passed { serde_json::json!(180.0) } else { serde_json::Value::Null },
        "avg_latency_ms": if passed { 840.0 } else { 0.0 },
        "prompt_tokens_avg": 128.0,
        "output_tokens_avg": 96.0,
        "total_tokens_avg": 224.0,
        "total_duration_ms": if passed { 2520.0 } else { 0.0 },
        "passed": passed,
        "warnings": if passed { vec!["deterministic contract fixture".to_string()] } else { vec!["real llm benchmark skipped or failed".to_string()] },
        "errors": if passed { Vec::<String>::new() } else { vec!["offline fixture".to_string()] },
        "run_details": if passed {
            vec![
                serde_json::json!({"ttft_ms": 175.0, "tps": 44.1, "latency_ms": 810.0, "prompt_tokens": 128, "output_tokens": 96}),
                serde_json::json!({"ttft_ms": 181.0, "tps": 42.3, "latency_ms": 840.0, "prompt_tokens": 128, "output_tokens": 96}),
                serde_json::json!({"ttft_ms": 184.0, "tps": 39.5, "latency_ms": 870.0, "prompt_tokens": 128, "output_tokens": 96})
            ]
        } else {
            Vec::<serde_json::Value>::new()
        }
    }));
    synthetic_signed_report(report)
}

pub(crate) fn provider_details() -> BurdProviderDetails {
    let system = system_report();
    let score = score_report();
    let pricing = calculate_pricing(&system, &score);
    let uptime = uptime_summary();
    let network = network_score_report();
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
        session: None,
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
        uptime_score: uptime.uptime_score,
        reliability_score: 85.6,
        network_score: network.network_score,
        uptime,
        reliability: reliability_report(),
        network,
        capability_spot: capability_spot_report(),
        workload_eligibility: workload_eligibility_report(),
        stats: provider_stats(),
        pricing: pricing.clone(),
        tier: score.tier.clone(),
        score,
        heartbeat: None,
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

pub(crate) fn capability_spot_report() -> CapabilitySpotVerificationReport {
    crate::capability::calculate_capability_spot_verification(
        &system_report(),
        &fit_report(),
        &provider_verification(),
        Some(&signed_report_with_llm_benchmark(true)),
        Some(&history_list()),
    )
}


pub(crate) fn trust_score_report() -> TrustScoreReport {
    crate::trust::calculate_trust_score(
        &provider_verification(),
        &reliability_report(),
        &network_score_report(),
        Some(&history_list()),
        None,
    )
}

pub(crate) fn workload_eligibility_report() -> WorkloadEligibilityReport {
    crate::workload::calculate_workload_eligibility(
        &system_report(),
        &fit_report(),
        &score_report(),
        &provider_verification(),
        &reliability_report(),
        &capability_spot_report(),
        &trust_score_report(),
        Some(&history_list()),
    )
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
        uptime_score: 100.0,
        uptime_level: "Excellent".to_string(),
        last_online_at: Some(FIXTURE_TIMESTAMP.to_string()),
        last_failed_check_at: None,
        checks_total: 1,
        checks_failed: 0,
        current_status: "idle".to_string(),
    }
}

pub(crate) fn reliability_report() -> ReliabilityReport {
    ReliabilityReport {
        reliability_score: 85.6,
        uptime_score: 100.0,
        level: "Good".to_string(),
        status: "warming_up".to_string(),
        components: ReliabilityComponents {
            uptime_1d: 100.0,
            uptime_7d: 100.0,
            uptime_30d: 100.0,
            sample_coverage: 4.2,
            latest_status: 100.0,
            failure_penalty: 0.0,
        },
        uptime: uptime_summary(),
        checks_total: 1,
        checks_failed: 0,
        consecutive_failed_checks: 0,
        warnings: vec!["fewer than 3 heartbeat checks; reliability score is warming up".to_string()],
        notes: vec![
            "Local reliability is derived from local heartbeat history only.".to_string(),
            "Uptime score weights: 50% 1d, 30% 7d, 20% 30d.".to_string(),
            "Reliability score weights: 70% uptime score, 15% sample coverage, 15% latest status, minus consecutive failure penalty.".to_string(),
            "Reliability score is not backend availability, audit approval, marketplace admission, or a payout guarantee.".to_string(),
        ],
    }
}

fn network_benchmark_report() -> NetworkBenchmarkReport {
    NetworkBenchmarkReport {
        endpoint: "https://www.cloudflare.com/cdn-cgi/trace".to_string(),
        attempts: 5,
        latency_avg_ms: Some(80.0),
        latency_min_ms: Some(76.0),
        latency_max_ms: Some(84.0),
        avg_latency_ms: Some(80.0),
        min_latency_ms: Some(76.0),
        max_latency_ms: Some(84.0),
        jitter_ms: Some(5.0),
        successful_requests: 5,
        failed_requests: 0,
        failures: 0,
        loss_pct: 0.0,
        status_code: Some(200),
        dns_resolution_ms: Some(10.0),
        download_probe_bytes: None,
        duration_ms: 410.0,
        passed: true,
        warnings: vec![],
        errors: vec![],
    }
}

pub(crate) fn network_score_report() -> NetworkScoreReport {
    calculate_network_score(Some(&network_benchmark_report()))
}

pub(crate) fn history_list() -> BenchmarkHistoryList {
    BenchmarkHistoryList {
        path: "<path>".to_string(),
        entries_total: 3,
        entries: vec![BenchmarkHistoryEntry {
            history_id: "history-fixture".to_string(),
            timestamp: FIXTURE_TIMESTAMP.to_string(),
            agent_version: "0.1.0-test".to_string(),
            benchmark_version: BENCHMARK_VERSION.to_string(),
            provider_id: Some(FIXTURE_PROVIDER_ID.to_string()),
            machine_id: Some(FIXTURE_MACHINE_ID.to_string()),
            benchmark_profile: "profile_24gb".to_string(),
            system_summary: SystemSummary {
                os: Some("linux".to_string()),
                architecture: Some("x86_64".to_string()),
                cpu: Some("Burd Contract CPU".to_string()),
                cpu_cores: Some(16),
                ram_total_gb: Some(64.0),
                backend_detected: Some("CUDA".to_string()),
            },
            gpu_summary: vec![GpuSummary {
                name: "NVIDIA GeForce RTX 4090".to_string(),
                vram_gb: Some(24.0),
                backend: "CUDA".to_string(),
                count: 1,
            }],
            score: 82.5,
            tier: "Burd Pro".to_string(),
            llm_benchmark_summary: serde_json::json!({"status": "skipped"}),
            stability_summary: serde_json::json!({"status": "skipped"}),
            network_summary: serde_json::json!({"status": "passed"}),
            disk_summary: serde_json::json!({"status": "skipped"}),
            report_hash: "sha256:fixture".to_string(),
            signed: true,
            challenge_id: Some("challenge-contract".to_string()),
            verification_status: "signature_valid_locally".to_string(),
            warnings: Vec::new(),
        }],
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







