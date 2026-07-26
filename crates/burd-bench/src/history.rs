use burd_hardware::SystemReport;
use burd_protocol::{
    FullReport, SignedReport, default_state_dir, hash_canonical, write_json_atomic,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkHistoryEntry {
    pub history_id: String,
    pub timestamp: String,
    pub agent_version: String,
    pub benchmark_version: String,
    pub provider_id: Option<String>,
    pub machine_id: Option<String>,
    pub benchmark_profile: String,
    pub system_summary: SystemSummary,
    pub gpu_summary: Vec<GpuSummary>,
    pub score: f64,
    pub tier: String,
    pub llm_benchmark_summary: serde_json::Value,
    pub stability_summary: serde_json::Value,
    pub network_summary: serde_json::Value,
    pub disk_summary: serde_json::Value,
    pub report_hash: String,
    pub signed: bool,
    pub challenge_id: Option<String>,
    pub verification_status: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSummary {
    pub os: Option<String>,
    pub architecture: Option<String>,
    pub cpu: Option<String>,
    pub cpu_cores: Option<u64>,
    pub ram_total_gb: Option<f64>,
    pub backend_detected: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuSummary {
    pub name: String,
    pub vram_gb: Option<f64>,
    pub backend: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkHistoryList {
    pub path: String,
    pub entries_total: usize,
    pub entries: Vec<BenchmarkHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkHistoryLatest {
    pub path: String,
    pub entries_total: usize,
    pub latest: Option<BenchmarkHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkHistoryClearResult {
    pub path: String,
    pub cleared: bool,
    pub entries_removed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkHistoryExportResult {
    pub output: String,
    pub entries_exported: usize,
}

pub fn append_report_history(report: &FullReport) -> Result<BenchmarkHistoryEntry, String> {
    let report_hash = hash_canonical(report)?;
    append_history_entry(entry_from_report(report, &report_hash, false, "unsigned"))
}

pub fn append_signed_report_history(
    signed_report: &SignedReport,
) -> Result<BenchmarkHistoryEntry, String> {
    append_history_entry(entry_from_report(
        &signed_report.report,
        &signed_report.report_hash,
        true,
        if signed_report.signature_valid_locally {
            "signature_valid_locally"
        } else {
            "signature_invalid_locally"
        },
    ))
}

pub fn load_history_list() -> Result<BenchmarkHistoryList, String> {
    let entries = load_history_entries()?;
    Ok(BenchmarkHistoryList {
        path: history_path().display().to_string(),
        entries_total: entries.len(),
        entries,
    })
}

pub fn load_latest_history() -> Result<BenchmarkHistoryLatest, String> {
    let entries = load_history_entries()?;
    Ok(BenchmarkHistoryLatest {
        path: history_path().display().to_string(),
        entries_total: entries.len(),
        latest: entries.last().cloned(),
    })
}

pub fn clear_history(confirm: bool) -> Result<BenchmarkHistoryClearResult, String> {
    if !confirm {
        return Err("history clear requires --confirm".to_string());
    }
    let path = history_path();
    let removed = load_history_entries()
        .map(|entries| entries.len())
        .unwrap_or(0);
    write_json_atomic(&path, &Vec::<BenchmarkHistoryEntry>::new())
        .map_err(|error| format!("failed to clear benchmark history: {error}"))?;
    Ok(BenchmarkHistoryClearResult {
        path: path.display().to_string(),
        cleared: true,
        entries_removed: removed,
    })
}

pub fn export_history(output: &Path) -> Result<BenchmarkHistoryExportResult, String> {
    let entries = load_history_entries()?;
    if let Some(dir) = output.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(&entries)
        .map_err(|error| format!("failed to serialize benchmark history: {error}"))?;
    fs::write(output, json)
        .map_err(|error| format!("failed to write {}: {error}", output.display()))?;
    Ok(BenchmarkHistoryExportResult {
        output: output.display().to_string(),
        entries_exported: entries.len(),
    })
}

pub fn history_summary() -> serde_json::Value {
    match load_history_entries() {
        Ok(entries) => serde_json::json!({
            "entries_total": entries.len(),
            "latest": entries.last(),
        }),
        Err(error) => serde_json::json!({
            "entries_total": 0,
            "latest": null,
            "error": error,
        }),
    }
}

fn append_history_entry(entry: BenchmarkHistoryEntry) -> Result<BenchmarkHistoryEntry, String> {
    let mut entries = load_history_entries()?;
    entries.push(entry.clone());
    save_history_entries(&entries)?;
    Ok(entry)
}

fn load_history_entries() -> Result<Vec<BenchmarkHistoryEntry>, String> {
    let path = history_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).map_err(|error| {
        format!(
            "benchmark history not readable at {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_str(&raw).map_err(|error| {
        format!(
            "invalid benchmark history JSON at {}: {error}",
            path.display()
        )
    })
}

fn save_history_entries(entries: &[BenchmarkHistoryEntry]) -> Result<(), String> {
    let path = history_path();
    write_json_atomic(&path, entries)
        .map_err(|error| format!("failed to persist benchmark history: {error}"))
}

fn entry_from_report(
    report: &FullReport,
    report_hash: &str,
    signed: bool,
    verification_status: &str,
) -> BenchmarkHistoryEntry {
    let system = report
        .system
        .clone()
        .as_object()
        .cloned()
        .unwrap_or_default();
    let score = report.score.clone();
    let warnings = collect_warnings(report);
    let hash_prefix = report_hash.chars().take(12).collect::<String>();
    BenchmarkHistoryEntry {
        history_id: format!("history-{}-{hash_prefix}", Utc::now().timestamp_millis()),
        timestamp: report.timestamp.clone(),
        agent_version: report.agent_version.clone(),
        benchmark_version: report.benchmark_version.clone(),
        provider_id: report
            .identity
            .as_ref()
            .map(|identity| identity.provider_id.clone()),
        machine_id: report
            .identity
            .as_ref()
            .map(|identity| identity.machine_id.clone()),
        benchmark_profile: report.benchmark_profile.clone(),
        system_summary: system_summary_from_value(&system),
        gpu_summary: gpu_summary_from_report(report),
        score: json_f64(&score, "burd_compute_score").unwrap_or(0.0),
        tier: json_string(&score, "tier").unwrap_or_else(|| "unknown".to_string()),
        llm_benchmark_summary: llm_summary(report.llm_benchmark.as_ref()),
        stability_summary: stability_summary(report.stability.as_ref()),
        network_summary: network_summary(report.network.as_ref()),
        disk_summary: disk_summary(report.disk.as_ref()),
        report_hash: report_hash.to_string(),
        signed,
        challenge_id: report
            .challenge
            .as_ref()
            .map(|challenge| challenge.challenge_id.clone()),
        verification_status: verification_status.to_string(),
        warnings,
    }
}

fn collect_warnings(report: &FullReport) -> Vec<String> {
    let mut warnings = Vec::new();
    for value in [
        Some(&report.score),
        report.llm_benchmark.as_ref(),
        report.stability.as_ref(),
        report.network.as_ref(),
        report.disk.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(items) = value.get("warnings").and_then(|item| item.as_array()) {
            warnings.extend(
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(ToOwned::to_owned),
            );
        }
    }
    warnings.sort();
    warnings.dedup();
    warnings
}

fn system_summary_from_value(system: &serde_json::Map<String, serde_json::Value>) -> SystemSummary {
    SystemSummary {
        os: map_string(system, "os"),
        architecture: map_string(system, "architecture"),
        cpu: map_string(system, "cpu"),
        cpu_cores: system.get("cpu_cores").and_then(|value| value.as_u64()),
        ram_total_gb: system.get("ram_total_gb").and_then(|value| value.as_f64()),
        backend_detected: map_string(system, "backend_detected"),
    }
}

fn gpu_summary_from_report(report: &FullReport) -> Vec<GpuSummary> {
    serde_json::from_value::<SystemReport>(report.system.clone())
        .map(|system| {
            system
                .gpus
                .into_iter()
                .map(|gpu| GpuSummary {
                    name: gpu.name,
                    vram_gb: gpu.vram_gb,
                    backend: gpu.backend,
                    count: gpu.count,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn llm_summary(value: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::json!({"status": "missing"});
    };
    serde_json::json!({
        "status": value.get("status").and_then(|item| item.as_str()),
        "provider": value.get("provider").and_then(|item| item.as_str()),
        "model": value.get("model").and_then(|item| item.as_str()),
        "runs": value.get("runs").and_then(|item| item.as_u64()),
        "avg_tps": value.get("avg_tps").and_then(|item| item.as_f64()),
        "avg_ttft_ms": value.get("avg_ttft_ms").and_then(|item| item.as_f64()),
        "passed": value.get("passed").and_then(|item| item.as_bool()),
        "errors_count": value.get("errors").and_then(|item| item.as_array()).map(|items| items.len()).unwrap_or(0),
    })
}

fn stability_summary(value: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::json!({"status": "missing"});
    };
    serde_json::json!({
        "status": value.get("status").and_then(|item| item.as_str()),
        "duration_seconds": value.get("duration_seconds").and_then(|item| item.as_u64()),
        "total_runs": value.get("total_runs").and_then(|item| item.as_u64()),
        "failed_runs": value.get("failed_runs").and_then(|item| item.as_u64()),
        "avg_tps": value.get("avg_tps").and_then(|item| item.as_f64()),
        "passed": value.get("passed").and_then(|item| item.as_bool()),
    })
}

fn network_summary(value: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::json!({"status": "missing"});
    };
    serde_json::json!({
        "status": value.get("status").and_then(|item| item.as_str()),
        "endpoint": value.get("endpoint").and_then(|item| item.as_str()),
        "latency_avg_ms": value.get("latency_avg_ms").or_else(|| value.get("avg_latency_ms")).and_then(|item| item.as_f64()),
        "latency_min_ms": value.get("latency_min_ms").or_else(|| value.get("min_latency_ms")).and_then(|item| item.as_f64()),
        "latency_max_ms": value.get("latency_max_ms").or_else(|| value.get("max_latency_ms")).and_then(|item| item.as_f64()),
        "jitter_ms": value.get("jitter_ms").and_then(|item| item.as_f64()),
        "successful_requests": value.get("successful_requests").and_then(|item| item.as_u64()),
        "failed_requests": value.get("failed_requests").or_else(|| value.get("failures")).and_then(|item| item.as_u64()),
        "passed": value.get("passed").and_then(|item| item.as_bool()),
    })
}

fn disk_summary(value: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::json!({"status": "missing"});
    };
    serde_json::json!({
        "status": value.get("status").and_then(|item| item.as_str()),
        "free_space_gb": value.get("free_space_gb").and_then(|item| item.as_f64()),
        "sequential_read_mb_s": value.get("sequential_read_mb_s").and_then(|item| item.as_f64()),
        "sequential_write_mb_s": value.get("sequential_write_mb_s").and_then(|item| item.as_f64()),
        "passed": value.get("passed").and_then(|item| item.as_bool()),
    })
}

fn json_f64(value: &serde_json::Value, key: &str) -> Option<f64> {
    value.get(key).and_then(|item| item.as_f64())
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(ToOwned::to_owned)
}

fn map_string(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(|item| item.as_str())
        .map(ToOwned::to_owned)
}

fn history_path() -> PathBuf {
    default_state_dir().join("benchmark-history.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::ReportSignature;

    #[test]
    fn history_entry_does_not_include_private_key() {
        let report = FullReport {
            identity: None,
            evidence: None,
            hardware_fingerprint: None,
            marketplace_policy: None,
            system: serde_json::json!({
                "os": "linux",
                "architecture": "x86_64",
                "cpu": "cpu",
                "cpu_cores": 8,
                "ram_total_gb": 32.0,
                "backend_detected": "CUDA",
                "gpus": [],
                "gpu_count": 0,
                "ram_available_gb": 16.0,
                "primary_gpu_name": null,
                "vram_per_gpu_gb": null,
                "vram_total_gb": null,
                "cuda_available": false,
                "rocm_available": false,
                "nvidia_driver": null,
                "amd_driver": null,
                "container_detected": false,
                "vm_detected": false,
                "timestamp": "2026-06-08T00:00:00Z",
                "agent_version": "0.1.0",
                "benchmark_version": "test"
            }),
            fit: None,
            llm_benchmark: None,
            stability: None,
            network: None,
            network_score: None,
            disk: None,
            reliability: None,
            ai_performance: None,
            score: serde_json::json!({"burd_compute_score": 80.0, "tier": "Burd Pro"}),
            timestamp: "2026-06-08T00:00:00Z".to_string(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: "test".to_string(),
            benchmark_profile: "profile_24gb".to_string(),
            challenge: None,
            signature: ReportSignature {
                algorithm: "placeholder".to_string(),
                value: "placeholder".to_string(),
                status: "mocked".to_string(),
            },
        };
        let entry = entry_from_report(&report, "abc", false, "unsigned");
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("private_key"));
        assert_eq!(entry.score, 80.0);
    }
}
