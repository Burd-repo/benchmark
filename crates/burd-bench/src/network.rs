use serde::{Deserialize, Serialize};
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
}
