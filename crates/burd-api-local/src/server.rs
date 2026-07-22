use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use burd_bench::{
    ReportRunOptions, append_report_history, append_signed_report_history,
    build_ai_performance_report, build_capability_spot_verification, build_provider_details,
    build_provider_readiness, build_raw_data, build_registration_payload, build_trust_score,
    build_workload_eligibility, calculate_network_score, calculate_pricing, calculate_reliability,
    calculate_score, estimate_earnings, generate_full_report, generate_signed_report, load_actions,
    load_history_list, load_logs, load_network_score_report, load_reliability_report,
    load_uptime_summary, record_action, save_latest_report, verify_provider,
};
use burd_hardware::{build_system_report, detect_specs, detect_system_report};
use burd_llmfit::build_fit_report;
use burd_protocol::{
    Challenge, ChallengeResponse, challenge_response_message_with_fingerprint,
    evidence_freshness_from_window, load_identity, load_private_key, mock_challenge,
    redacted_config_value, sign_message, verify_api_token, verify_challenge_response,
};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::RwLock;

static UI_INDEX: &str = include_str!("../../../apps/benchmark-ui/index.html");
static UI_STYLES: &str = include_str!("../../../apps/benchmark-ui/styles.css");

#[derive(Debug)]
struct AppState {
    agent_version: String,
    host_uri: String,
    host: String,
    auth_warning: Option<String>,
    benchmark_status: RwLock<BenchmarkStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchmarkStatus {
    status: String,
    last_report: Option<serde_json::Value>,
}

pub fn run_server(host: &str, port: u16, agent_version: &str) -> Result<(), String> {
    let ip: IpAddr = host
        .parse()
        .map_err(|_| format!("invalid host '{host}', expected an IP address"))?;
    let addr = SocketAddr::new(ip, port);
    let state = Arc::new(AppState {
        agent_version: agent_version.to_string(),
        host_uri: format!("http://{host}:{port}"),
        host: host.to_string(),
        auth_warning: api_auth_warning(host),
        benchmark_status: RwLock::new(BenchmarkStatus {
            status: "idle".to_string(),
            last_report: None,
        }),
    });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to create tokio runtime: {error}"))?;

    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|error| format!("failed to bind {addr}: {error}"))?;
        axum::serve(listener, router(state))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|error| format!("server error: {error}"))
    })
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/styles.css", get(styles))
        .route("/health", get(health))
        .route("/api/v1/system", get(system))
        .route("/api/v1/fit", get(fit))
        .route("/api/v1/score", get(score))
        .route("/api/v1/report", get(report))
        .route("/api/v1/report/signed", post(signed_report))
        .route("/api/v1/challenge/mock", get(challenge_mock))
        .route("/api/v1/provider", get(provider))
        .route("/api/v1/readiness", get(readiness))
        .route("/api/v1/verification", get(verification))
        .route("/api/v1/uptime", get(uptime))
        .route("/api/v1/reliability", get(reliability))
        .route("/api/v1/network-score", get(network_score))
        .route("/api/v1/trust-score", get(trust_score))
        .route("/api/v1/ai-performance", get(ai_performance))
        .route("/api/v1/capability-spot", get(capability_spot))
        .route("/api/v1/workload-eligibility", get(workload_eligibility))
        .route("/api/v1/history", get(history))
        .route("/api/v1/registration-payload", get(registration_payload))
        .route("/api/v1/pricing", get(pricing))
        .route("/api/v1/earnings", get(earnings))
        .route("/api/v1/actions", get(actions))
        .route("/api/v1/logs", get(logs))
        .route("/api/v1/raw", get(raw))
        .route("/api/v1/config", get(config))
        .route("/api/v1/benchmark/run", post(run_benchmark))
        .route("/api/v1/challenge/run", post(run_challenge))
        .route("/api/v1/benchmark/status", get(benchmark_status))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(UI_INDEX)
}

async fn styles() -> impl IntoResponse {
    ([(CONTENT_TYPE, "text/css; charset=utf-8")], UI_STYLES)
}

async fn health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "agent_version": state.agent_version,
        "api_auth_warning": state.auth_warning.clone(),
    }))
}

async fn system(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(detect_system_report(
        &state.agent_version
    )))
}

async fn fit() -> Json<serde_json::Value> {
    let specs = detect_specs();
    Json(serde_json::json!(build_fit_report(&specs, Some(25))))
}

async fn score(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let specs = detect_specs();
    let system = build_system_report(&specs, &state.agent_version);
    let fit = build_fit_report(&specs, Some(25));
    let score = calculate_score(&system, Some(&fit), None, None, None, None);
    Json(serde_json::json!(score))
}

async fn report(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(response) = auth_failure(&state, &headers) {
        return response;
    }
    let report = generate_full_report(ReportRunOptions::new(state.agent_version.clone()));
    ok_json(serde_json::json!(report))
}

async fn signed_report(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = auth_failure(&state, &headers) {
        return response;
    }
    let options = ReportRunOptions::new(state.agent_version.clone());
    match generate_signed_report(options) {
        Ok(report) => ok_json(serde_json::json!(report)),
        Err(error) => ok_json(serde_json::json!({
            "status": "failed",
            "error": error,
        })),
    }
}

async fn challenge_mock() -> Json<serde_json::Value> {
    Json(serde_json::json!(mock_challenge("profile_8gb")))
}

async fn provider(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(build_provider_details(
        &state.agent_version,
        &state.host_uri
    )))
}

async fn readiness(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(build_provider_readiness(
        &state.agent_version,
        &state.host_uri
    )))
}

async fn verification(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(verify_provider(&state.agent_version)))
}

async fn uptime() -> Json<serde_json::Value> {
    Json(serde_json::json!(load_uptime_summary().unwrap_or_else(
        |_| burd_bench::UptimeSummary {
            uptime_1d: 0.0,
            uptime_7d: 0.0,
            uptime_30d: 0.0,
            uptime_score: 0.0,
            uptime_level: "No Data".to_string(),
            last_online_at: None,
            last_failed_check_at: None,
            checks_total: 0,
            checks_failed: 0,
            current_status: "unknown".to_string(),
        }
    )))
}

async fn reliability() -> Json<serde_json::Value> {
    Json(serde_json::json!(
        load_reliability_report().unwrap_or_else(|_| calculate_reliability(&[]))
    ))
}

async fn network_score() -> Json<serde_json::Value> {
    Json(serde_json::json!(
        load_network_score_report().unwrap_or_else(|_| calculate_network_score(None))
    ))
}

async fn trust_score(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(build_trust_score(&state.agent_version)))
}

async fn ai_performance(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(build_ai_performance_report(
        &state.agent_version
    )))
}

async fn capability_spot(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(build_capability_spot_verification(
        &state.agent_version
    )))
}

async fn workload_eligibility(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(build_workload_eligibility(
        &state.agent_version
    )))
}

async fn history() -> Json<serde_json::Value> {
    Json(serde_json::json!(load_history_list().unwrap_or_else(
        |_| burd_bench::BenchmarkHistoryList {
            path: String::new(),
            entries_total: 0,
            entries: Vec::new(),
        }
    )))
}

async fn registration_payload(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(build_registration_payload(
        &state.agent_version
    )))
}

async fn pricing(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let (system, score) = system_and_score(&state.agent_version);
    Json(serde_json::json!(calculate_pricing(&system, &score)))
}

async fn earnings(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let (system, score) = system_and_score(&state.agent_version);
    let pricing = calculate_pricing(&system, &score);
    Json(serde_json::json!(estimate_earnings(&pricing)))
}

async fn actions() -> Json<serde_json::Value> {
    Json(serde_json::json!(load_actions().unwrap_or_default()))
}

async fn logs() -> Json<serde_json::Value> {
    Json(serde_json::json!(load_logs().unwrap_or_default()))
}

async fn raw(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(response) = auth_failure(&state, &headers) {
        return response;
    }
    ok_json(serde_json::json!(build_raw_data(
        &state.agent_version,
        &state.host_uri
    )))
}

async fn config(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(response) = auth_failure(&state, &headers) {
        return response;
    }
    ok_json(match redacted_config_value() {
        Ok(config) => config,
        Err(error) => serde_json::json!({
            "status": "failed",
            "error": error,
        }),
    })
}

async fn run_benchmark(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = auth_failure(&state, &headers) {
        return response;
    }
    {
        let mut status = state.benchmark_status.write().await;
        status.status = "running".to_string();
        status.last_report = None;
    }

    let mut options = ReportRunOptions::new(state.agent_version.clone());
    options.run_all = true;
    let report = generate_full_report(options);
    let _ = save_latest_report(&report);
    let _ = append_report_history(&report);
    let _ = record_action(
        "report generation",
        "completed",
        "Run benchmark from API",
        "Generated local provider validation report from API request.",
        vec!["api benchmark completed".to_string()],
    );
    let report_json = serde_json::json!(report);

    {
        let mut status = state.benchmark_status.write().await;
        status.status = "completed".to_string();
        status.last_report = Some(report_json.clone());
    }

    ok_json(serde_json::json!({
        "status": "completed",
        "report": report_json,
    }))
}

async fn run_challenge(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(challenge): Json<Challenge>,
) -> impl IntoResponse {
    if let Some(response) = auth_failure(&state, &headers) {
        return response;
    }
    let mut options = ReportRunOptions::new(state.agent_version.clone());
    options.run_all = true;
    options.challenge = Some(challenge.clone());
    let signed_report = match generate_signed_report(options) {
        Ok(report) => report,
        Err(error) => {
            return ok_json(serde_json::json!({
                "status": "failed",
                "error": error,
            }));
        }
    };
    let _ = save_latest_report(&signed_report.report);
    let _ = append_signed_report_history(&signed_report);
    let config = match load_identity() {
        Ok(config) => config,
        Err(error) => {
            return ok_json(serde_json::json!({
                "status": "failed",
                "error": error,
            }));
        }
    };
    let private_key = match load_private_key(&config) {
        Ok(key) => key,
        Err(error) => {
            return ok_json(serde_json::json!({
                "status": "failed",
                "error": error,
            }));
        }
    };
    let hardware_fingerprint = match signed_report.report.hardware_fingerprint.clone() {
        Some(fingerprint) => fingerprint,
        None => {
            return ok_json(serde_json::json!({
                "status": "failed",
                "error": "signed report does not include hardware fingerprint",
            }));
        }
    };
    let message = match challenge_response_message_with_fingerprint(
        &challenge.challenge_id,
        &challenge.nonce,
        &config.provider_id,
        &config.machine_id,
        &signed_report.report_hash,
        &hardware_fingerprint,
    ) {
        Ok(message) => message,
        Err(error) => {
            return ok_json(serde_json::json!({
                "status": "failed",
                "error": error,
            }));
        }
    };
    let signature = match sign_message(&private_key.secret_key_base64, message.as_bytes()) {
        Ok(signature) => signature,
        Err(error) => {
            return ok_json(serde_json::json!({
                "status": "failed",
                "error": error,
            }));
        }
    };
    let completed_at = chrono::Utc::now().to_rfc3339();
    let response_evidence =
        match evidence_freshness_from_window(&challenge.issued_at, &challenge.expires_at) {
            Ok(evidence) => evidence,
            Err(error) => {
                return ok_json(serde_json::json!({
                    "status": "failed",
                    "error": error,
                }));
            }
        };
    let mut response = ChallengeResponse {
        challenge_id: challenge.challenge_id.clone(),
        nonce: challenge.nonce.clone(),
        provider_id: config.provider_id,
        machine_id: config.machine_id,
        report_hash: signed_report.report_hash.clone(),
        hardware_fingerprint: Some(hardware_fingerprint),
        signed_report: Some(signed_report.clone()),
        signature,
        public_key: signed_report.public_key.clone(),
        completed_at,
        issued_at: response_evidence.issued_at,
        expires_at: response_evidence.expires_at,
        is_expired: response_evidence.is_expired,
        age_seconds: response_evidence.age_seconds,
        ttl_seconds: response_evidence.ttl_seconds,
        status: "partial".to_string(),
        failed_requirements: Vec::new(),
        verification_result: None,
    };
    let verification = verify_challenge_response(&challenge, &response);
    response.status = if verification.expired {
        "expired".to_string()
    } else if verification.valid {
        "passed".to_string()
    } else {
        "failed".to_string()
    };
    response.failed_requirements = verification.errors.clone();
    response.verification_result = Some(serde_json::json!({
        "valid": verification.valid,
        "signature_valid": verification.signature_valid,
        "expired": verification.expired,
        "evidence": verification.evidence.clone(),
        "checked_at": verification.checked_at.clone(),
        "warnings": verification.warnings.clone(),
        "errors": verification.errors.clone(),
    }));
    let _ = record_action(
        "challenge response",
        if verification.valid {
            "completed"
        } else {
            "failed"
        },
        "Run challenge from API",
        "Ran local challenge and signed response from API request.",
        verification.errors.clone(),
    );
    ok_json(serde_json::json!({
        "status": response.status.clone(),
        "challenge": challenge,
        "signed_report": signed_report,
        "response": response,
        "verification": verification,
    }))
}

async fn benchmark_status(State(state): State<Arc<AppState>>) -> Json<BenchmarkStatus> {
    Json(state.benchmark_status.read().await.clone())
}

fn auth_failure(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    if !api_auth_required() {
        return None;
    }

    let token = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let valid = token
        .map(|token| verify_api_token(token).unwrap_or(false))
        .unwrap_or(false);
    if valid {
        None
    } else {
        Some((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "status": "unauthorized",
                "error": "missing or invalid API token",
                "hint": "send Authorization: Bearer <token>",
                "host": state.host.clone(),
            })),
        ))
    }
}
fn ok_json(value: serde_json::Value) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::OK, Json(value))
}

fn api_auth_warning(host: &str) -> Option<String> {
    let config_path = burd_protocol::default_config_path();
    if !config_path.exists() {
        return if host == "127.0.0.1" || host == "::1" || host == "localhost" {
            Some("dev mode: local API token is not configured on loopback".to_string())
        } else {
            Some(
                "strong warning: local API token is not configured while binding beyond loopback"
                    .to_string(),
            )
        };
    }

    match load_identity() {
        Ok(config) if config.api_auth_enabled && config.api_token_hash.is_some() => None,
        Ok(_) => {
            if host == "127.0.0.1" || host == "::1" || host == "localhost" {
                Some("dev mode: local API token is not configured on loopback".to_string())
            } else {
                Some(
                    "strong warning: local API token is not configured while binding beyond loopback"
                        .to_string(),
                )
            }
        }
        Err(_) => Some(
            "strong warning: local API token configuration exists but could not be loaded; protected endpoints fail closed"
                .to_string(),
        ),
    }
}
fn api_auth_required() -> bool {
    let config_path = burd_protocol::default_config_path();
    if !config_path.exists() {
        return false;
    }

    load_identity()
        .map(|config| config.api_auth_enabled && config.api_token_hash.is_some())
        .unwrap_or(true)
}
fn system_and_score(agent_version: &str) -> (burd_hardware::SystemReport, burd_bench::ScoreReport) {
    let specs = detect_specs();
    let system = build_system_report(&specs, agent_version);
    let fit = build_fit_report(&specs, Some(25));
    let score = calculate_score(&system, Some(&fit), None, None, None, None);
    (system, score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use burd_protocol::sha256_hex;
    use serde_json::Value;
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    const TEST_AGENT_VERSION: &str = "test-agent";

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState {
            agent_version: TEST_AGENT_VERSION.to_string(),
            host_uri: "http://127.0.0.1:8787".to_string(),
            host: "127.0.0.1".to_string(),
            auth_warning: Some(
                "dev mode: local API token is not configured on loopback".to_string(),
            ),
            benchmark_status: RwLock::new(BenchmarkStatus {
                status: "idle".to_string(),
                last_report: None,
            }),
        })
    }

    async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.get_or_init(|| Mutex::new(())).lock().await
    }

    async fn request_json(method: Method, path: &str) -> (StatusCode, Value) {
        request_json_with_auth(method, path, None).await
    }

    async fn request_json_with_auth(
        method: Method,
        path: &str,
        token: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = router(test_state())
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 10 * 1024 * 1024)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes).unwrap();
        (status, value)
    }

    async fn get_json(path: &str) -> (StatusCode, Value) {
        request_json(Method::GET, path).await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn health_endpoint_contract_is_stable() {
        let (status, value) = get_json("/health").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["status"], "ok");
        assert_eq!(value["agent_version"], TEST_AGENT_VERSION);
        assert!(value.get("api_auth_warning").is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lightweight_public_endpoints_keep_contracts() {
        let _lock = env_lock().await;
        let _env = TestEnv::new(false);
        let cases = [
            (
                "/api/v1/uptime",
                vec!["uptime_1d", "uptime_7d", "uptime_30d", "checks_total"],
            ),
            (
                "/api/v1/reliability",
                vec!["reliability_score", "uptime_score", "status"],
            ),
            (
                "/api/v1/network-score",
                vec!["network_score", "level", "status", "components"],
            ),
            (
                "/api/v1/ai-performance",
                vec!["status", "level", "source", "confidence_level"],
            ),
            (
                "/api/v1/trust-score",
                vec!["trust_score", "level", "status", "components"],
            ),
            (
                "/api/v1/capability-spot",
                vec!["capability_score", "level", "status", "checks"],
            ),
            (
                "/api/v1/workload-eligibility",
                vec!["local_status", "marketplace_status_future", "workloads"],
            ),
            ("/api/v1/history", vec!["entries_total", "entries"]),
            (
                "/api/v1/readiness",
                vec![
                    "status",
                    "readiness_score",
                    "readiness_level",
                    "checks",
                    "warnings",
                    "recommendations",
                ],
            ),
            ("/api/v1/actions", vec![]),
            ("/api/v1/logs", vec![]),
            (
                "/api/v1/challenge/mock",
                vec!["challenge_id", "nonce", "policy"],
            ),
            ("/api/v1/benchmark/status", vec!["status", "last_report"]),
        ];

        for (path, keys) in cases {
            let (status, value) = get_json(path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            for key in keys {
                assert!(value.get(key).is_some(), "{path} missing {key}");
            }
        }
    }

    #[test]
    fn provider_console_ui_consumes_pr11_contracts_without_auto_heavy_actions() {
        assert!(UI_INDEX.contains("/api/v1/ai-performance"));
        assert!(UI_INDEX.contains("/api/v1/trust-score"));
        assert!(UI_INDEX.contains("/api/v1/capability-spot"));
        assert!(UI_INDEX.contains("/api/v1/workload-eligibility"));
        assert!(UI_INDEX.contains("data-tab=\"workloads\""));
        assert!(UI_INDEX.contains("Local Trust Assessment"));
        assert!(UI_INDEX.contains("Capability Spot - Local/Mock"));
        assert!(UI_INDEX.contains("future_marketplace"));
        assert!(UI_INDEX.contains("Token required"));
        assert!(UI_INDEX.contains("window.confirm"));
        assert!(UI_INDEX.contains("manual heavy operation"));
        assert!(!UI_INDEX.contains("Marketplace Approved"));
        assert!(!UI_INDEX.contains("Remote Verified"));
        assert!(!UI_INDEX.contains("Global Trust"));
        assert!(!UI_INDEX.contains("state.raw = await postJson(\"/api/v1/benchmark/run\")"));
    }

    #[test]
    fn provider_console_ui_redacts_secrets_and_marks_local_future_states() {
        assert!(UI_INDEX.contains("sanitizeSecrets"));
        assert!(UI_INDEX.contains("private_key"));
        assert!(UI_INDEX.contains("api_token_hash"));
        assert!(UI_INDEX.contains("local heuristic"));
        assert!(UI_INDEX.contains("local/mock"));
        assert!(UI_INDEX.contains("Not measured"));
        assert!(UI_INDEX.contains("No history; not enough samples yet and not treated as fraud."));
        assert!(UI_INDEX.contains("Online locally"));
        assert!(!UI_INDEX.contains("api-token-placeholder-secret-value"));
        assert!(!UI_INDEX.contains("private-key-placeholder-secret-value"));
    }
    #[tokio::test(flavor = "current_thread")]
    async fn protected_endpoints_return_token_contract_when_auth_enabled() {
        let _lock = env_lock().await;
        let env = TestEnv::new(true);
        env.write_auth_config();
        let cases = [
            (Method::GET, "/api/v1/report"),
            (Method::POST, "/api/v1/report/signed"),
            (Method::GET, "/api/v1/raw"),
            (Method::GET, "/api/v1/config"),
        ];

        for (method, path) in cases {
            let (status, value) = request_json(method, path).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
            assert_eq!(value["status"], "unauthorized");
            assert_eq!(value["error"], "missing or invalid API token");
            assert_eq!(value["hint"], "send Authorization: Bearer <token>");
            assert_eq!(value["host"], "127.0.0.1");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn protected_endpoint_accepts_valid_token_and_rejects_invalid_token() {
        let _lock = env_lock().await;
        let env = TestEnv::new(true);
        env.write_auth_config();

        let (missing_status, missing_value) = request_json(Method::GET, "/api/v1/config").await;
        assert_eq!(missing_status, StatusCode::UNAUTHORIZED);
        assert_eq!(missing_value["status"], "unauthorized");

        let (invalid_status, invalid_value) =
            request_json_with_auth(Method::GET, "/api/v1/config", Some("invalid-token")).await;
        assert_eq!(invalid_status, StatusCode::UNAUTHORIZED);
        assert_eq!(invalid_value["status"], "unauthorized");

        let (valid_status, valid_value) =
            request_json_with_auth(Method::GET, "/api/v1/config", Some(&env.api_token)).await;
        assert_eq!(valid_status, StatusCode::OK);
        assert_eq!(valid_value["private_key_path"], "[redacted]");
        assert_eq!(valid_value["api_token_hash"], "[redacted]");
        let serialized = serde_json::to_string(&valid_value).unwrap();
        assert!(!serialized.contains(&env.api_token));
        assert!(!serialized.contains(&env.api_token_hash));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn protected_endpoint_fails_closed_when_auth_config_is_corrupted() {
        let _lock = env_lock().await;
        let env = TestEnv::new(true);
        env.write_auth_config();
        fs::write(&env.config_path, "{ not valid json").unwrap();

        let (status, value) = request_json(Method::GET, "/api/v1/config").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(value["status"], "unauthorized");
        assert_eq!(value["error"], "missing or invalid API token");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn api_auth_warning_reports_corrupted_config() {
        let _lock = env_lock().await;
        let env = TestEnv::new(true);
        env.write_auth_config();
        fs::write(&env.config_path, "{ not valid json").unwrap();

        let warning = api_auth_warning("127.0.0.1").unwrap();
        assert!(warning.contains("could not be loaded"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn redacted_config_does_not_expose_secret_values() {
        let _lock = env_lock().await;
        let env = TestEnv::new(true);
        env.write_auth_config();
        let value = redacted_config_value().unwrap();
        let serialized = serde_json::to_string(&value).unwrap();

        assert_eq!(value["private_key_path"], "[redacted]");
        assert_eq!(value["api_token_hash"], "[redacted]");
        assert!(!serialized.contains(env.private_key_path.to_str().unwrap()));
        assert!(!serialized.contains(env.api_token_hash.as_str()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn raw_data_fixture_keeps_redaction_contract() {
        let raw = serde_json::json!({
            "redacted": true,
            "redacted_fields": [
                "private_key",
                "secret_key_base64",
                "private_key_path",
                "api_token",
                "api_token_hash",
                "credentials"
            ],
            "config_redacted": {
                "provider_id": "provider",
                "private_key_path": "[redacted]",
                "api_token_hash": "[redacted]"
            },
            "latest_signed_report_summary": {
                "report_hash": "hash",
                "signature_valid_locally": true
            }
        });
        let serialized = serde_json::to_string(&raw).unwrap();

        assert_eq!(raw["redacted"], true);
        assert!(!serialized.contains("super-secret-token"));
        assert!(!serialized.contains("secret-key-material"));
    }

    struct TestEnv {
        previous_home: Option<OsString>,
        previous_config: Option<OsString>,
        state_dir: PathBuf,
        config_path: PathBuf,
        private_key_path: PathBuf,
        api_token: String,
        api_token_hash: String,
    }

    impl TestEnv {
        fn new(api_auth_enabled: bool) -> Self {
            let state_dir = unique_temp_dir(api_auth_enabled);
            fs::create_dir_all(&state_dir).unwrap();
            let config_path = state_dir.join("agent.json");
            let private_key_path = state_dir.join("agent.key");
            let api_token = "burd-api-local-test-token".to_string();
            let api_token_hash = sha256_hex(api_token.as_bytes());
            let previous_home = std::env::var_os("BURD_AGENT_HOME");
            let previous_config = std::env::var_os("BURD_AGENT_CONFIG");

            // SAFETY: these tests serialize Burd env var mutation through ENV_LOCK,
            // and no background threads are spawned by the test body.
            unsafe {
                std::env::set_var("BURD_AGENT_HOME", &state_dir);
                std::env::set_var("BURD_AGENT_CONFIG", &config_path);
            }

            Self {
                previous_home,
                previous_config,
                state_dir,
                config_path,
                private_key_path,
                api_token,
                api_token_hash,
            }
        }

        fn write_auth_config(&self) {
            let config = serde_json::json!({
                "provider_id": "burd-provider-test",
                "machine_id": "burd-machine-test",
                "api_url": "https://api.burd.cloud",
                "preferred_provider": "ollama",
                "benchmark_profile": "auto",
                "telemetry_enabled": false,
                "created_at": "2026-06-08T00:00:00Z",
                "public_key": "public-key",
                "key_algorithm": "ed25519",
                "private_key_path": self.private_key_path,
                "email": null,
                "website": null,
                "country": null,
                "city": null,
                "region": null,
                "api_token_hash": self.api_token_hash,
                "api_auth_enabled": true,
                "api_bind_host": "127.0.0.1",
                "api_port": 8787,
                "default_network_endpoint": "https://www.cloudflare.com/cdn-cgi/trace"
            });
            fs::write(
                &self.config_path,
                serde_json::to_string_pretty(&config).unwrap(),
            )
            .unwrap();
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            // SAFETY: tests that mutate these env vars are serialized through ENV_LOCK.
            unsafe {
                if let Some(value) = &self.previous_home {
                    std::env::set_var("BURD_AGENT_HOME", value);
                } else {
                    std::env::remove_var("BURD_AGENT_HOME");
                }

                if let Some(value) = &self.previous_config {
                    std::env::set_var("BURD_AGENT_CONFIG", value);
                } else {
                    std::env::remove_var("BURD_AGENT_CONFIG");
                }
            }
            let _ = fs::remove_dir_all(&self.state_dir);
        }
    }

    fn unique_temp_dir(api_auth_enabled: bool) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "burd-api-local-test-{}-{}-{nanos}",
            std::process::id(),
            if api_auth_enabled { "auth" } else { "empty" }
        ))
    }
}
