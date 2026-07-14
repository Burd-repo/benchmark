use chrono::Utc;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ObservabilityState {
    inner: Arc<ObservabilityInner>,
}

#[derive(Debug)]
struct ObservabilityInner {
    service: String,
    environment: String,
    deployment_id: String,
    started_at: String,
    started: Instant,
    recent_limit: usize,
    slo: ServiceSlo,
    metrics: Mutex<MetricsState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservabilitySettings {
    pub recent_events_limit: u32,
    pub availability_target_bps: u32,
    pub p95_latency_ms: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservabilitySnapshot {
    pub service: String,
    pub environment: String,
    pub deployment_id: String,
    pub started_at: String,
    pub uptime_seconds: u64,
    pub http: HttpMetricsSnapshot,
    pub background: BackgroundMetricsSnapshot,
    pub slo: SloSnapshot,
    pub recent_events: Vec<ObservedHttpEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpMetricsSnapshot {
    pub in_flight: u64,
    pub total_requests: u64,
    pub total_errors: u64,
    pub rate_limited: u64,
    pub status_2xx: u64,
    pub status_3xx: u64,
    pub status_4xx: u64,
    pub status_5xx: u64,
    pub total_duration_ms: u128,
    pub max_duration_ms: u128,
    pub average_duration_ms: Option<f64>,
    pub recent_p95_duration_ms: Option<u128>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundMetricsSnapshot {
    pub total_errors: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SloSnapshot {
    pub availability_target_bps: u32,
    pub p95_latency_target_ms: u32,
    pub current_availability_bps: Option<u32>,
    pub recent_p95_latency_ms: Option<u128>,
    pub availability_status: String,
    pub latency_status: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ServiceSlo {
    pub availability_target_bps: u32,
    pub p95_latency_ms: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservedHttpEvent {
    pub timestamp: String,
    pub correlation_id: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u128,
}

#[derive(Debug, Clone)]
pub struct ObservedHttpRequest {
    pub correlation_id: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u128,
}

#[derive(Debug, Default)]
struct MetricsState {
    in_flight: u64,
    total_requests: u64,
    total_errors: u64,
    rate_limited: u64,
    status_2xx: u64,
    status_3xx: u64,
    status_4xx: u64,
    status_5xx: u64,
    total_duration_ms: u128,
    max_duration_ms: u128,
    background_errors: u64,
    recent_events: VecDeque<ObservedHttpEvent>,
}

impl ObservabilityState {
    pub fn new(
        service: impl Into<String>,
        environment: impl Into<String>,
        deployment_id: impl Into<String>,
        settings: ObservabilitySettings,
    ) -> Self {
        let slo = ServiceSlo {
            availability_target_bps: settings.availability_target_bps.clamp(1, 10_000),
            p95_latency_ms: settings.p95_latency_ms,
        };
        Self {
            inner: Arc::new(ObservabilityInner {
                service: service.into(),
                environment: environment.into(),
                deployment_id: deployment_id.into(),
                started_at: Utc::now().to_rfc3339(),
                started: Instant::now(),
                recent_limit: settings.recent_events_limit.max(1) as usize,
                slo,
                metrics: Mutex::new(MetricsState::default()),
            }),
        }
    }

    pub fn begin_http_request(&self) {
        let mut metrics = self.metrics();
        metrics.in_flight = metrics.in_flight.saturating_add(1);
    }

    pub fn finish_http_request(&self, observed: ObservedHttpRequest) {
        let event = ObservedHttpEvent {
            timestamp: Utc::now().to_rfc3339(),
            correlation_id: observed.correlation_id,
            method: observed.method,
            path: observed.path,
            status: observed.status,
            duration_ms: observed.duration_ms,
        };
        let mut metrics = self.metrics();
        metrics.in_flight = metrics.in_flight.saturating_sub(1);
        metrics.total_requests = metrics.total_requests.saturating_add(1);
        metrics.total_duration_ms = metrics.total_duration_ms.saturating_add(event.duration_ms);
        metrics.max_duration_ms = metrics.max_duration_ms.max(event.duration_ms);
        match event.status {
            200..=299 => metrics.status_2xx = metrics.status_2xx.saturating_add(1),
            300..=399 => metrics.status_3xx = metrics.status_3xx.saturating_add(1),
            400..=499 => metrics.status_4xx = metrics.status_4xx.saturating_add(1),
            500..=599 => metrics.status_5xx = metrics.status_5xx.saturating_add(1),
            _ => {}
        }
        if event.status >= 500 {
            metrics.total_errors = metrics.total_errors.saturating_add(1);
        }
        if event.status == 429 {
            metrics.rate_limited = metrics.rate_limited.saturating_add(1);
        }
        metrics.recent_events.push_back(event.clone());
        while metrics.recent_events.len() > self.inner.recent_limit {
            metrics.recent_events.pop_front();
        }
        drop(metrics);

        log_json(
            "http_request",
            serde_json::json!({
                "correlation_id": event.correlation_id,
                "method": event.method,
                "path": event.path,
                "status": event.status,
                "duration_ms": event.duration_ms,
            }),
        );
    }

    pub fn record_background_task_error(&self, task: &str, error: impl ToString) {
        let mut metrics = self.metrics();
        metrics.background_errors = metrics.background_errors.saturating_add(1);
        drop(metrics);
        log_json(
            "background_task_error",
            serde_json::json!({
                "task": task,
                "error": error.to_string(),
            }),
        );
    }

    pub fn snapshot(&self) -> ObservabilitySnapshot {
        let metrics = self.metrics();
        let recent_events = metrics.recent_events.iter().cloned().collect::<Vec<_>>();
        let recent_p95_duration_ms = percentile_duration_ms(&recent_events, 95);
        let current_availability_bps =
            availability_bps(metrics.total_requests, metrics.total_errors);
        let http = HttpMetricsSnapshot {
            in_flight: metrics.in_flight,
            total_requests: metrics.total_requests,
            total_errors: metrics.total_errors,
            rate_limited: metrics.rate_limited,
            status_2xx: metrics.status_2xx,
            status_3xx: metrics.status_3xx,
            status_4xx: metrics.status_4xx,
            status_5xx: metrics.status_5xx,
            total_duration_ms: metrics.total_duration_ms,
            max_duration_ms: metrics.max_duration_ms,
            average_duration_ms: average_duration_ms(
                metrics.total_duration_ms,
                metrics.total_requests,
            ),
            recent_p95_duration_ms,
        };
        let background = BackgroundMetricsSnapshot {
            total_errors: metrics.background_errors,
        };
        drop(metrics);

        let slo = SloSnapshot {
            availability_target_bps: self.inner.slo.availability_target_bps,
            p95_latency_target_ms: self.inner.slo.p95_latency_ms,
            current_availability_bps,
            recent_p95_latency_ms: recent_p95_duration_ms,
            availability_status: slo_status(
                current_availability_bps.map(u128::from),
                u128::from(self.inner.slo.availability_target_bps),
            ),
            latency_status: slo_status_max(
                recent_p95_duration_ms,
                u128::from(self.inner.slo.p95_latency_ms),
            ),
        };

        ObservabilitySnapshot {
            service: self.inner.service.clone(),
            environment: self.inner.environment.clone(),
            deployment_id: self.inner.deployment_id.clone(),
            started_at: self.inner.started_at.clone(),
            uptime_seconds: self.inner.started.elapsed().as_secs(),
            http,
            background,
            slo,
            recent_events,
        }
    }

    pub fn prometheus(&self) -> String {
        let snapshot = self.snapshot();
        let labels = format!(
            "service=\"{}\",environment=\"{}\",deployment_id=\"{}\"",
            escape_label(&snapshot.service),
            escape_label(&snapshot.environment),
            escape_label(&snapshot.deployment_id)
        );
        let availability = snapshot
            .slo
            .current_availability_bps
            .map(|value| f64::from(value) / 10_000.0)
            .unwrap_or(1.0);
        let average_ms = snapshot.http.average_duration_ms.unwrap_or(0.0);
        let p95_ms = snapshot.slo.recent_p95_latency_ms.unwrap_or(0);
        format!(
            concat!(
                "# HELP burd_control_plane_uptime_seconds Control plane process uptime in seconds.\n",
                "# TYPE burd_control_plane_uptime_seconds gauge\n",
                "burd_control_plane_uptime_seconds{{{labels}}} {uptime}\n",
                "# HELP burd_control_plane_http_requests_total Total HTTP requests observed by the control plane.\n",
                "# TYPE burd_control_plane_http_requests_total counter\n",
                "burd_control_plane_http_requests_total{{{labels}}} {requests}\n",
                "# HELP burd_control_plane_http_errors_total Total HTTP 5xx responses observed by the control plane.\n",
                "# TYPE burd_control_plane_http_errors_total counter\n",
                "burd_control_plane_http_errors_total{{{labels}}} {errors}\n",
                "# HELP burd_control_plane_http_in_flight_requests Current in-flight HTTP requests.\n",
                "# TYPE burd_control_plane_http_in_flight_requests gauge\n",
                "burd_control_plane_http_in_flight_requests{{{labels}}} {in_flight}\n",
                "# HELP burd_control_plane_http_request_duration_average_ms Average observed HTTP request duration in milliseconds.\n",
                "# TYPE burd_control_plane_http_request_duration_average_ms gauge\n",
                "burd_control_plane_http_request_duration_average_ms{{{labels}}} {average_ms:.3}\n",
                "# HELP burd_control_plane_http_request_duration_recent_p95_ms Recent in-memory p95 HTTP request duration in milliseconds.\n",
                "# TYPE burd_control_plane_http_request_duration_recent_p95_ms gauge\n",
                "burd_control_plane_http_request_duration_recent_p95_ms{{{labels}}} {p95_ms}\n",
                "# HELP burd_control_plane_slo_availability_ratio Current availability ratio from HTTP 5xx responses.\n",
                "# TYPE burd_control_plane_slo_availability_ratio gauge\n",
                "burd_control_plane_slo_availability_ratio{{{labels}}} {availability:.5}\n",
                "# HELP burd_control_plane_background_errors_total Background task errors observed by the control plane.\n",
                "# TYPE burd_control_plane_background_errors_total counter\n",
                "burd_control_plane_background_errors_total{{{labels}}} {background_errors}\n"
            ),
            labels = labels,
            uptime = snapshot.uptime_seconds,
            requests = snapshot.http.total_requests,
            errors = snapshot.http.total_errors,
            in_flight = snapshot.http.in_flight,
            average_ms = average_ms,
            p95_ms = p95_ms,
            availability = availability,
            background_errors = snapshot.background.total_errors,
        )
    }

    fn metrics(&self) -> std::sync::MutexGuard<'_, MetricsState> {
        self.inner
            .metrics
            .lock()
            .expect("observability mutex poisoned")
    }
}

pub fn log_json(event: &str, fields: serde_json::Value) {
    eprintln!(
        "{}",
        serde_json::json!({
            "event": event,
            "timestamp": Utc::now().to_rfc3339(),
            "fields": fields,
        })
    );
}

pub fn normalize_http_path(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }
    let normalized = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            if looks_like_identifier(segment) {
                "{id}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("/{normalized}")
}

fn looks_like_identifier(segment: &str) -> bool {
    segment.contains('_')
        || segment.len() >= 24
        || segment
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
            && segment.len() >= 12
}

fn average_duration_ms(total: u128, count: u64) -> Option<f64> {
    (count > 0).then(|| total as f64 / count as f64)
}

fn availability_bps(total: u64, errors: u64) -> Option<u32> {
    if total == 0 {
        return None;
    }
    let successful = total.saturating_sub(errors);
    Some(((successful as u128 * 10_000) / u128::from(total)) as u32)
}

fn percentile_duration_ms(events: &[ObservedHttpEvent], percentile: u32) -> Option<u128> {
    if events.is_empty() {
        return None;
    }
    let mut durations = events
        .iter()
        .map(|event| event.duration_ms)
        .collect::<Vec<_>>();
    durations.sort_unstable();
    let rank = ((durations.len() as u128 * u128::from(percentile)).div_ceil(100)).saturating_sub(1)
        as usize;
    durations.get(rank.min(durations.len() - 1)).copied()
}

fn slo_status(current: Option<u128>, target: u128) -> String {
    match current {
        Some(value) if value >= target => "satisfied".to_string(),
        Some(_) => "violated".to_string(),
        None => "insufficient_data".to_string(),
    }
}

fn slo_status_max(current: Option<u128>, target: u128) -> String {
    match current {
        Some(value) if value <= target => "satisfied".to_string(),
        Some(_) => "violated".to_string(),
        None => "insufficient_data".to_string(),
    }
}

fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ObservabilityState {
        ObservabilityState::new(
            "burd-control-plane",
            "test",
            "deployment_1",
            ObservabilitySettings {
                recent_events_limit: 4,
                availability_target_bps: 9_990,
                p95_latency_ms: 500,
            },
        )
    }

    #[test]
    fn records_http_metrics_and_slo_status() {
        let observability = state();
        observability.begin_http_request();
        observability.finish_http_request(ObservedHttpRequest {
            correlation_id: "corr_1".to_string(),
            method: "GET".to_string(),
            path: "/health".to_string(),
            status: 200,
            duration_ms: 10,
        });
        observability.begin_http_request();
        observability.finish_http_request(ObservedHttpRequest {
            correlation_id: "corr_2".to_string(),
            method: "GET".to_string(),
            path: "/ready".to_string(),
            status: 503,
            duration_ms: 20,
        });

        let snapshot = observability.snapshot();
        assert_eq!(snapshot.http.total_requests, 2);
        assert_eq!(snapshot.http.total_errors, 1);
        assert_eq!(snapshot.http.status_2xx, 1);
        assert_eq!(snapshot.http.status_5xx, 1);
        assert_eq!(snapshot.slo.availability_status, "violated");
        assert!(
            observability
                .prometheus()
                .contains("burd_control_plane_http_requests_total")
        );
    }

    #[test]
    fn normalizes_high_cardinality_paths() {
        assert_eq!(
            normalize_http_path("/v1/providers/provider_123/jobs/job_456"),
            "/v1/providers/{id}/jobs/{id}"
        );
        assert_eq!(normalize_http_path("/health"), "/health");
    }
}
