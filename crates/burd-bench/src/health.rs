use burd_hardware::{BENCHMARK_VERSION, SystemReport, detect_system_report};
use burd_protocol::default_state_dir;
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
    pub health: HealthReport,
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
    let health = detect_health(agent_version);
    let mut history = load_uptime_history().unwrap_or_default();
    history.checks.push(UptimeCheck {
        checked_at: health.last_seen_at.clone(),
        online: health.online,
        status: health.current_status.clone(),
    });
    if history.checks.len() > 20_000 {
        let keep_from = history.checks.len() - 20_000;
        history.checks.drain(0..keep_from);
    }
    save_uptime_history(&history)?;
    Ok(HeartbeatReport {
        recorded: true,
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
