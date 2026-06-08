use crate::disk::{DiskBenchmarkOptions, run_disk_benchmark};
use crate::llm::{LlmBenchmarkOptions, LlmBenchmarkReport, run_llm_benchmark};
use crate::network::{NetworkBenchmarkOptions, run_network_benchmark};
use crate::profiles::profile_for_vram;
use crate::score::calculate_score;
use crate::stability::{StabilityBenchmarkReport, run_stability_benchmark};
use burd_hardware::{BENCHMARK_VERSION, detect_specs, detect_system_report};
use burd_llmfit::build_fit_report;
use burd_protocol::{FullReport, load_identity, placeholder_signature};
use chrono::Utc;

#[derive(Debug, Clone)]
pub struct ReportRunOptions {
    pub run_all: bool,
    pub agent_version: String,
    pub llm_provider: String,
    pub llm_url: Option<String>,
    pub llm_model: Option<String>,
}

impl ReportRunOptions {
    pub fn new(agent_version: impl Into<String>) -> Self {
        Self {
            run_all: false,
            agent_version: agent_version.into(),
            llm_provider: "ollama".to_string(),
            llm_url: None,
            llm_model: None,
        }
    }
}

pub fn generate_full_report(options: ReportRunOptions) -> FullReport {
    let specs = detect_specs();
    let system = detect_system_report(&options.agent_version);
    let vram = system
        .vram_total_gb
        .or(system.vram_per_gpu_gb)
        .unwrap_or(0.0);
    let profile = profile_for_vram(vram);
    let fit = build_fit_report(&specs, Some(25));
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
        Some(&fit),
        llm_benchmark.as_ref(),
        stability.as_ref(),
        network.as_ref(),
        disk.as_ref(),
    );

    let machine_id = identity.as_ref().map(|value| value.machine_id.clone());
    FullReport {
        identity,
        system: serde_json::to_value(&system).expect("system report serializes"),
        fit: Some(serde_json::to_value(&fit).expect("fit report serializes")),
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
        timestamp: Utc::now().to_rfc3339(),
        agent_version: options.agent_version,
        benchmark_version: BENCHMARK_VERSION.to_string(),
        benchmark_profile: profile.id,
        challenge: None,
        signature: placeholder_signature(machine_id.as_deref(), None),
    }
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
}
