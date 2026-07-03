use burd_protocol::default_state_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkBenchmarkOptions {
    pub endpoint: String,
    pub attempts: usize,
}

impl Default for NetworkBenchmarkOptions {
    fn default() -> Self {
        Self {
            endpoint: "https://www.cloudflare.com/cdn-cgi/trace".to_string(),
            attempts: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkBenchmarkReport {
    pub endpoint: String,
    pub attempts: usize,
    pub latency_avg_ms: Option<f64>,
    pub latency_min_ms: Option<f64>,
    pub latency_max_ms: Option<f64>,
    pub avg_latency_ms: Option<f64>,
    pub min_latency_ms: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub successful_requests: usize,
    pub failed_requests: usize,
    pub failures: usize,
    pub loss_pct: f64,
    pub status_code: Option<u16>,
    pub dns_resolution_ms: Option<f64>,
    pub download_probe_bytes: Option<u64>,
    pub duration_ms: f64,
    pub passed: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkScoreReport {
    pub network_score: f64,
    pub level: String,
    pub status: String,
    pub source: String,
    pub components: NetworkScoreComponents,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmark: Option<NetworkBenchmarkReport>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkScoreComponents {
    pub latency_score: f64,
    pub jitter_score: f64,
    pub loss_score: f64,
    pub dns_score: f64,
    pub success_rate: f64,
    pub avg_latency_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_pct: f64,
    pub dns_resolution_ms: Option<f64>,
}

pub fn run_network_benchmark(options: NetworkBenchmarkOptions) -> NetworkBenchmarkReport {
    let attempts = options.attempts.max(1);
    let total_start = Instant::now();
    let dns_resolution_ms = dns_resolution_ms(&options.endpoint);
    let mut latencies = Vec::new();
    let mut errors = Vec::new();
    let mut status_code = None;

    for _ in 0..attempts {
        let start = Instant::now();
        let response = ureq::get(&options.endpoint)
            .config()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .call();
        match response {
            Ok(response) => {
                status_code = Some(response.status().as_u16());
                latencies.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            Err(error) => errors.push(error.to_string()),
        }
    }

    let mut report = summarize_network(options.endpoint, attempts, latencies, errors);
    report.status_code = status_code;
    report.dns_resolution_ms = dns_resolution_ms.map(round1);
    report.duration_ms = round1(total_start.elapsed().as_secs_f64() * 1000.0);
    report
}

pub fn save_latest_network_benchmark(report: &NetworkBenchmarkReport) -> Result<(), String> {
    let path = latest_network_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialize latest network benchmark: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn load_latest_network_benchmark() -> Result<NetworkBenchmarkReport, String> {
    let path = latest_network_path();
    let raw = fs::read_to_string(&path).map_err(|error| {
        format!(
            "latest network benchmark not found at {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_str(&raw).map_err(|error| format!("invalid latest network JSON: {error}"))
}

pub fn load_network_score_report() -> Result<NetworkScoreReport, String> {
    let network_path = latest_network_path();
    if network_path.exists() {
        let benchmark = load_latest_network_benchmark()?;
        return Ok(calculate_network_score(Some(&benchmark)));
    }

    let path = default_state_dir().join("latest-report.json");
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("latest report not found at {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid latest report JSON: {error}"))?;
    network_score_from_report_value(&value)
}

pub fn network_score_from_report_value(
    value: &serde_json::Value,
) -> Result<NetworkScoreReport, String> {
    if let Some(score) = value.get("network_score")
        && !score.is_null()
    {
        return serde_json::from_value(score.clone())
            .map_err(|error| format!("invalid network score JSON: {error}"));
    }

    let Some(network) = value.get("network") else {
        return Ok(calculate_network_score(None));
    };
    if network
        .get("status")
        .and_then(|status| status.as_str())
        .is_some_and(|status| status == "skipped")
    {
        return Ok(calculate_network_score(None));
    }
    let benchmark: NetworkBenchmarkReport = serde_json::from_value(network.clone())
        .map_err(|error| format!("invalid network benchmark JSON: {error}"))?;
    Ok(calculate_network_score(Some(&benchmark)))
}

pub fn calculate_network_score(report: Option<&NetworkBenchmarkReport>) -> NetworkScoreReport {
    let Some(report) = report else {
        return NetworkScoreReport {
            network_score: 0.0,
            level: "No Data".to_string(),
            status: "no_benchmark".to_string(),
            source: "latest-report.json".to_string(),
            components: NetworkScoreComponents {
                latency_score: 0.0,
                jitter_score: 0.0,
                loss_score: 0.0,
                dns_score: 0.0,
                success_rate: 0.0,
                avg_latency_ms: None,
                jitter_ms: None,
                loss_pct: 100.0,
                dns_resolution_ms: None,
            },
            benchmark: None,
            warnings: vec![
                "network benchmark not available; run `burd-agent bench network --json` or `burd-agent report --run-all --json` to collect a finite local sample".to_string(),
            ],
            notes: network_score_notes(),
        };
    };

    let avg_latency_ms = report.avg_latency_ms.or(report.latency_avg_ms);
    let jitter_ms = report.jitter_ms;
    let dns_resolution_ms = report.dns_resolution_ms;
    let attempts = report
        .attempts
        .max(report.successful_requests + report.failed_requests)
        .max(report.successful_requests + report.failures);
    let success_rate = if attempts == 0 {
        0.0
    } else {
        report.successful_requests as f64 / attempts as f64 * 100.0
    };

    let latency_score = avg_latency_ms
        .map(|latency| (100.0 - latency / 4.0).clamp(0.0, 100.0))
        .unwrap_or(0.0);
    let jitter_score = jitter_ms
        .map(|jitter| (100.0 - jitter * 2.0).clamp(0.0, 100.0))
        .unwrap_or(0.0);
    let loss_score = (100.0 - report.loss_pct * 5.0).clamp(0.0, 100.0);
    let dns_score = dns_resolution_ms
        .map(|dns| (100.0 - dns / 2.0).clamp(0.0, 100.0))
        .unwrap_or(60.0);

    let mut score =
        latency_score * 0.45 + jitter_score * 0.20 + loss_score * 0.25 + dns_score * 0.10;
    if !report.passed {
        score = score.min(55.0);
    }
    let score = round1(score.clamp(0.0, 100.0));
    let mut warnings = report.warnings.clone();
    if !report.passed {
        warnings.push("network benchmark did not pass local thresholds".to_string());
    }
    if attempts < 3 {
        warnings.push("fewer than 3 network attempts; network score is warming up".to_string());
    }
    if avg_latency_ms.is_some_and(|latency| latency > 150.0) {
        warnings.push("average latency is above 150 ms".to_string());
    }
    if jitter_ms.is_some_and(|jitter| jitter > 30.0) {
        warnings.push("jitter is above 30 ms".to_string());
    }
    if report.loss_pct > 0.0 {
        warnings.push(format!(
            "packet/request loss observed: {:.1}%",
            report.loss_pct
        ));
    }
    warnings.sort();
    warnings.dedup();

    NetworkScoreReport {
        network_score: score,
        level: network_level(score).to_string(),
        status: network_status(score, report.passed).to_string(),
        source: report.endpoint.clone(),
        components: NetworkScoreComponents {
            latency_score: round1(latency_score),
            jitter_score: round1(jitter_score),
            loss_score: round1(loss_score),
            dns_score: round1(dns_score),
            success_rate: round1(success_rate),
            avg_latency_ms,
            jitter_ms,
            loss_pct: report.loss_pct,
            dns_resolution_ms,
        },
        benchmark: Some(report.clone()),
        warnings,
        notes: network_score_notes(),
    }
}

pub fn summarize_network(
    endpoint: String,
    attempts: usize,
    latencies: Vec<f64>,
    errors: Vec<String>,
) -> NetworkBenchmarkReport {
    let failures = attempts.saturating_sub(latencies.len());
    let loss_pct = if attempts == 0 {
        100.0
    } else {
        failures as f64 / attempts as f64 * 100.0
    };
    let avg = average(&latencies);
    let min = latencies.iter().copied().reduce(f64::min);
    let max = latencies.iter().copied().reduce(f64::max);
    let jitter = jitter(&latencies);
    let successful_requests = latencies.len();
    let mut warnings = Vec::new();
    if let Some(avg) = avg
        && avg > 200.0
    {
        warnings.push("average latency above 200 ms".to_string());
    }
    if failures > 0 {
        warnings.push(format!("{failures} network attempts failed"));
    }

    NetworkBenchmarkReport {
        endpoint,
        attempts,
        latency_avg_ms: avg.map(round1),
        latency_min_ms: min.map(round1),
        latency_max_ms: max.map(round1),
        avg_latency_ms: avg.map(round1),
        min_latency_ms: min.map(round1),
        max_latency_ms: max.map(round1),
        jitter_ms: jitter.map(round1),
        successful_requests,
        failed_requests: failures,
        failures,
        loss_pct: round1(loss_pct),
        status_code: None,
        dns_resolution_ms: None,
        download_probe_bytes: None,
        duration_ms: 0.0,
        passed: !latencies.is_empty() && loss_pct <= 20.0,
        warnings,
        errors,
    }
}

fn dns_resolution_ms(endpoint: &str) -> Option<f64> {
    let host = endpoint
        .split("://")
        .nth(1)
        .unwrap_or(endpoint)
        .split('/')
        .next()
        .unwrap_or(endpoint)
        .split('@')
        .next_back()
        .unwrap_or(endpoint);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        return None;
    }
    let start = Instant::now();
    let _ = (host, 443).to_socket_addrs().ok()?;
    Some(start.elapsed().as_secs_f64() * 1000.0)
}

fn average(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

fn jitter(values: &[f64]) -> Option<f64> {
    if values.len() <= 1 {
        return Some(0.0);
    }
    let deltas: Vec<f64> = values
        .windows(2)
        .map(|window| (window[1] - window[0]).abs())
        .collect();
    average(&deltas)
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn network_status(score: f64, passed: bool) -> &'static str {
    if !passed || score < 40.0 {
        "failed"
    } else if score >= 90.0 {
        "excellent"
    } else if score >= 75.0 {
        "strong"
    } else if score >= 60.0 {
        "usable"
    } else {
        "constrained"
    }
}

fn network_level(score: f64) -> &'static str {
    match score {
        value if value <= 0.0 => "No Data",
        value if value >= 90.0 => "Excellent",
        value if value >= 75.0 => "Strong",
        value if value >= 60.0 => "Usable",
        value if value >= 40.0 => "Constrained",
        _ => "Poor",
    }
}

fn latest_network_path() -> std::path::PathBuf {
    default_state_dir().join("latest-network.json")
}

fn network_score_notes() -> Vec<String> {
    vec![
        "Local network score is derived from the latest finite network benchmark only.".to_string(),
        "Network score weights: 45% latency, 20% jitter, 25% loss/success, 10% DNS resolution."
            .to_string(),
        "Network score is not backend availability, public reachability, marketplace admission, or a payout guarantee."
            .to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_summary_passes_clean_low_latency() {
        let report = summarize_network(
            "http://example".to_string(),
            3,
            vec![10.0, 12.0, 11.0],
            vec![],
        );
        assert!(report.passed);
        assert_eq!(report.failures, 0);
    }

    #[test]
    fn network_score_rewards_low_latency_clean_sample() {
        let report = summarize_network(
            "http://example".to_string(),
            5,
            vec![20.0, 21.0, 19.0, 20.0, 21.0],
            vec![],
        );
        let score = calculate_network_score(Some(&report));

        assert!(score.network_score >= 90.0);
        assert_eq!(score.status, "excellent");
        assert_eq!(score.components.success_rate, 100.0);
    }

    #[test]
    fn network_score_reports_no_benchmark_without_running_network() {
        let score = calculate_network_score(None);

        assert_eq!(score.network_score, 0.0);
        assert_eq!(score.status, "no_benchmark");
        assert!(score.benchmark.is_none());
    }

    #[test]
    fn network_score_loader_reuses_report_score_when_network_is_skipped() {
        let score = calculate_network_score(None);
        let value = serde_json::json!({
            "network": {"status": "skipped"},
            "network_score": score,
        });

        let loaded = network_score_from_report_value(&value).unwrap();

        assert_eq!(loaded.status, "no_benchmark");
        assert_eq!(loaded.network_score, 0.0);
    }
}
