use crate::session::build_provider_session_status;
use burd_hardware::{
    BENCHMARK_VERSION, SystemReport, build_hardware_fingerprint_report, detect_system_report,
    gpu_vendor,
};
use burd_protocol::{
    ProviderHeartbeatSummary, ProviderSessionStatus, default_state_dir,
    heartbeat_summary_from_session, save_provider_session,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use sysinfo::Disks;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub online: bool,
    pub agent_version: String,
    pub benchmark_version: String,
    pub last_seen_at: String,
    pub gpu_available: bool,
    pub gpu_busy: bool,
    pub gpu_temperature_c: Option<f64>,
    pub gpu_utilization_pct: Option<f64>,
    pub vram_used_gb: Option<f64>,
    pub vram_total_gb: Option<f64>,
    pub disk_free_gb: Option<f64>,
    pub network_status: String,
    pub current_status: String,
    pub uptime: UptimeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UptimeCheck {
    pub checked_at: String,
    pub online: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UptimeHistory {
    pub checks: Vec<UptimeCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UptimeSummary {
    pub uptime_1d: f64,
    pub uptime_7d: f64,
    pub uptime_30d: f64,
    pub last_online_at: Option<String>,
    pub last_failed_check_at: Option<String>,
    pub checks_total: usize,
    pub checks_failed: usize,
    pub current_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatReport {
    pub recorded: bool,
    pub timestamp: String,
    pub provider_id: Option<String>,
    pub machine_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub session_status: Option<ProviderSessionStatus>,
    pub hardware_fingerprint: Option<String>,
    pub fingerprint_matches_session: bool,
    pub gpu_name: Option<String>,
    pub gpu_vendor: Option<String>,
    pub gpu_count: Option<u32>,
    pub vram_total_gb: Option<f64>,
    pub vram_used_gb: Option<f64>,
    pub vram_free_gb: Option<f64>,
    pub vram_source: Option<String>,
    pub vram_confidence: Option<String>,
    pub backend: Option<String>,
    pub cuda_available: Option<bool>,
    pub rocm_available: Option<bool>,
    pub vulkan_available: Option<bool>,
    pub online_locally: bool,
    pub current_gpu_load_percent: Option<f64>,
    pub current_cpu_load_percent: Option<f64>,
    pub memory_available_gb: Option<f64>,
    pub network_ok: Option<bool>,
    pub heartbeat_count: u64,
    pub heartbeat_summary: Option<ProviderHeartbeatSummary>,
    pub utilization: HeartbeatUtilizationSnapshot,
    pub warnings: Vec<String>,
    pub health: HealthReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatUtilizationSnapshot {
    pub current_gpu_load_percent: Option<f64>,
    pub vram_used_gb: Option<f64>,
    pub vram_free_gb: Option<f64>,
    pub current_cpu_load_percent: Option<f64>,
    pub memory_available_gb: Option<f64>,
    pub network_ok: Option<bool>,
}

pub fn detect_health(agent_version: &str) -> HealthReport {
    let system = detect_system_report(agent_version);
    detect_health_from_system(agent_version, &system)
}

pub(crate) fn detect_health_from_system(
    agent_version: &str,
    system: &SystemReport,
) -> HealthReport {
    let gpu_available = system.gpu_count > 0;
    let uptime = load_uptime_summary().unwrap_or_else(|_| summarize_checks(&[]));
    HealthReport {
        online: true,
        agent_version: agent_version.to_string(),
        benchmark_version: BENCHMARK_VERSION.to_string(),
        last_seen_at: Utc::now().to_rfc3339(),
        gpu_available,
        gpu_busy: false,
        gpu_temperature_c: None,
        gpu_utilization_pct: None,
        vram_used_gb: None,
        vram_total_gb: system.vram_total_gb.or(system.vram_per_gpu_gb),
        disk_free_gb: disk_free_gb(),
        network_status: "unknown".to_string(),
        current_status: if gpu_available {
            "idle".to_string()
        } else {
            "degraded".to_string()
        },
        uptime,
    }
}

pub fn heartbeat_once(agent_version: &str) -> Result<HeartbeatReport, String> {
    let session_status = build_provider_session_status(agent_version, "http://127.0.0.1:8787")?;
    let timestamp = Utc::now().to_rfc3339();

    let mut history = load_uptime_history_or_empty()?;
    let warnings = session_status.warnings.clone();

    let Some(session) = session_status.session.clone() else {
        record_uptime_check(&mut history, &timestamp, false, "no_session")?;
        return Err(format!(
            "provider session is {}; heartbeat requires an active local session",
            session_status.status.as_str()
        ));
    };

    if session_status.status != ProviderSessionStatus::Active || !session_status.online_locally {
        record_uptime_check(
            &mut history,
            &timestamp,
            false,
            session_status.status.as_str(),
        )?;
        return Err(format!(
            "provider session is {}; heartbeat requires an active local session",
            session_status.status.as_str()
        ));
    }

    let system = detect_heartbeat_system_report(agent_version);
    let fingerprint = build_hardware_fingerprint_report(&system);
    let fingerprint_matches_session =
        fingerprint.hardware_fingerprint == session.hardware_fingerprint;
    if !fingerprint_matches_session {
        let mut invalidated = session.clone();
        invalidated.status = ProviderSessionStatus::Invalidated;
        invalidated.online_locally = false;
        invalidated.is_expired = false;
        invalidated.last_heartbeat_at = timestamp.clone();
        invalidated.last_heartbeat_status = Some("fingerprint_mismatch".to_string());
        invalidated.last_heartbeat_error =
            Some("hardware fingerprint changed since the session started".to_string());
        invalidated.last_heartbeat_fingerprint_matches_session = Some(false);
        invalidated.last_heartbeat_warnings = vec!["fingerprint_mismatch".to_string()];
        save_provider_session(&invalidated)?;
        record_uptime_check(&mut history, &timestamp, false, "fingerprint_mismatch")?;
        return Err("hardware fingerprint changed since the session started".to_string());
    }

    let mut updated_session = session.clone();
    updated_session.last_heartbeat_at = timestamp.clone();
    updated_session.heartbeat_count = updated_session.heartbeat_count.saturating_add(1);
    updated_session.last_heartbeat_status = Some("ok".to_string());
    updated_session.last_heartbeat_error = None;
    updated_session.last_heartbeat_fingerprint_matches_session = Some(true);
    updated_session.last_heartbeat_warnings = Vec::new();
    updated_session.online_locally = true;
    save_provider_session(&updated_session)?;

    record_uptime_check(&mut history, &timestamp, true, "heartbeat_ok")?;

    let health = detect_health_from_system(agent_version, &system);
    let utilization = HeartbeatUtilizationSnapshot {
        current_gpu_load_percent: None,
        vram_used_gb: None,
        vram_free_gb: None,
        current_cpu_load_percent: None,
        memory_available_gb: None,
        network_ok: None,
    };
    let heartbeat_summary = heartbeat_summary_from_session(Some(&updated_session));
    Ok(HeartbeatReport {
        recorded: true,
        timestamp,
        provider_id: Some(updated_session.provider_id.clone()),
        machine_id: Some(updated_session.machine_id.clone()),
        provider_session_id: Some(updated_session.provider_session_id.clone()),
        session_status: Some(ProviderSessionStatus::Active),
        hardware_fingerprint: Some(fingerprint.hardware_fingerprint),
        fingerprint_matches_session: true,
        gpu_name: system.primary_gpu_name.clone(),
        gpu_vendor: system.gpus.first().map(|gpu| gpu_vendor(&gpu.name)),
        gpu_count: Some(system.gpu_count),
        vram_total_gb: system.vram_total_gb.or(system.vram_per_gpu_gb),
        vram_used_gb: utilization.vram_used_gb,
        vram_free_gb: utilization.vram_free_gb,
        vram_source: system.vram_source.clone(),
        vram_confidence: system.vram_confidence.clone(),
        backend: Some(system.backend_detected.clone()),
        cuda_available: Some(system.cuda_available),
        rocm_available: Some(system.rocm_available),
        vulkan_available: Some(
            system
                .backend_detected
                .to_ascii_lowercase()
                .contains("vulkan"),
        ),
        online_locally: true,
        current_gpu_load_percent: utilization.current_gpu_load_percent,
        current_cpu_load_percent: utilization.current_cpu_load_percent,
        memory_available_gb: utilization.memory_available_gb,
        network_ok: utilization.network_ok,
        heartbeat_count: updated_session.heartbeat_count,
        heartbeat_summary,
        utilization,
        warnings,
        health: HealthReport {
            uptime: summarize_checks(&history.checks),
            ..health
        },
    })
}

pub fn load_uptime_summary() -> Result<UptimeSummary, String> {
    load_uptime_history()
        .map(|history| summarize_checks(&history.checks))
        .or_else(|error| {
            if error.contains("not found") {
                Ok(summarize_checks(&[]))
            } else {
                Err(error)
            }
        })
}

fn load_uptime_history_or_empty() -> Result<UptimeHistory, String> {
    load_uptime_history().or_else(|error| {
        if error.contains("not found") {
            Ok(UptimeHistory::default())
        } else {
            Err(error)
        }
    })
}

pub fn load_uptime_history() -> Result<UptimeHistory, String> {
    let path = uptime_path();
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("uptime history not found at {}: {error}", path.display()))?;
    serde_json::from_str(&raw).map_err(|error| format!("invalid uptime JSON: {error}"))
}

pub fn save_uptime_history(history: &UptimeHistory) -> Result<(), String> {
    let path = uptime_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(history)
        .map_err(|error| format!("failed to serialize uptime JSON: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn clear_uptime_history(confirm: bool) -> Result<UptimeSummary, String> {
    if !confirm {
        return Err("uptime clear requires --confirm".to_string());
    }
    save_uptime_history(&UptimeHistory { checks: Vec::new() })?;
    Ok(summarize_checks(&[]))
}

fn record_uptime_check(
    history: &mut UptimeHistory,
    checked_at: &str,
    online: bool,
    status: &str,
) -> Result<(), String> {
    history.checks.push(UptimeCheck {
        checked_at: checked_at.to_string(),
        online,
        status: status.to_string(),
    });
    if history.checks.len() > 20_000 {
        let keep_from = history.checks.len() - 20_000;
        history.checks.drain(0..keep_from);
    }
    save_uptime_history(history)
}

pub fn summarize_checks(checks: &[UptimeCheck]) -> UptimeSummary {
    let now = Utc::now();
    let failed = checks.iter().filter(|check| !check.online).count();
    let last_online = checks
        .iter()
        .rev()
        .find(|check| check.online)
        .map(|check| check.checked_at.clone());
    let last_failed = checks
        .iter()
        .rev()
        .find(|check| !check.online)
        .map(|check| check.checked_at.clone());
    let current_status = checks
        .last()
        .map(|check| {
            if check.online {
                check.status.clone()
            } else {
                "offline".to_string()
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    UptimeSummary {
        uptime_1d: ratio_for_window(checks, now - Duration::days(1)),
        uptime_7d: ratio_for_window(checks, now - Duration::days(7)),
        uptime_30d: ratio_for_window(checks, now - Duration::days(30)),
        last_online_at: last_online,
        last_failed_check_at: last_failed,
        checks_total: checks.len(),
        checks_failed: failed,
        current_status,
    }
}

fn ratio_for_window(checks: &[UptimeCheck], start: DateTime<Utc>) -> f64 {
    let in_window: Vec<&UptimeCheck> = checks
        .iter()
        .filter(|check| {
            DateTime::parse_from_rfc3339(&check.checked_at)
                .map(|date| date.with_timezone(&Utc) >= start)
                .unwrap_or(false)
        })
        .collect();
    if in_window.is_empty() {
        return 0.0;
    }
    let online = in_window.iter().filter(|check| check.online).count();
    round4(online as f64 / in_window.len() as f64)
}

fn disk_free_gb() -> Option<f64> {
    let disks = Disks::new_with_refreshed_list();
    let total = disks
        .iter()
        .map(|disk| disk.available_space() as f64)
        .sum::<f64>();
    if total <= 0.0 {
        None
    } else {
        Some(round2(total / 1_073_741_824.0))
    }
}

fn uptime_path() -> PathBuf {
    default_state_dir().join("uptime.json")
}

fn detect_heartbeat_system_report(agent_version: &str) -> burd_hardware::SystemReport {
    #[cfg(test)]
    {
        let mut system = crate::test_fixtures::system_report();
        system.agent_version = agent_version.to_string();
        system.timestamp = crate::test_fixtures::FIXTURE_TIMESTAMP.to_string();
        return system;
    }

    #[cfg(not(test))]
    {
        detect_system_report(agent_version)
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round4(value: f64) -> f64 {
    (value * 10000.0).round() / 10000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_calculates_ratios() {
        let now = Utc::now();
        let checks = vec![
            UptimeCheck {
                checked_at: (now - Duration::minutes(5)).to_rfc3339(),
                online: true,
                status: "idle".to_string(),
            },
            UptimeCheck {
                checked_at: (now - Duration::minutes(4)).to_rfc3339(),
                online: false,
                status: "failed".to_string(),
            },
            UptimeCheck {
                checked_at: (now - Duration::minutes(3)).to_rfc3339(),
                online: true,
                status: "idle".to_string(),
            },
        ];
        let summary = summarize_checks(&checks);
        assert_eq!(summary.checks_total, 3);
        assert_eq!(summary.checks_failed, 1);
        assert!(summary.uptime_1d > 0.66 && summary.uptime_1d < 0.67);
    }
}
