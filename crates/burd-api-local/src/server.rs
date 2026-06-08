use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use burd_bench::{
    ReportRunOptions, append_report_history, append_signed_report_history, build_provider_details,
    build_raw_data, build_registration_payload, calculate_pricing, calculate_score,
    estimate_earnings, generate_full_report, generate_signed_report, load_actions,
    load_history_list, load_logs, load_uptime_summary, record_action, save_latest_report,
    verify_provider,
};
use burd_hardware::{detect_specs, detect_system_report};
use burd_llmfit::build_fit_report;
use burd_protocol::{
    Challenge, ChallengeResponse, challenge_response_message, load_identity, load_private_key,
    mock_challenge, redacted_config_value, sign_message, verify_api_token,
    verify_challenge_response,
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
        .route("/api/v1/verification", get(verification))
        .route("/api/v1/uptime", get(uptime))
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
    let system = detect_system_report(&state.agent_version);
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

async fn verification(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(verify_provider(&state.agent_version)))
}

async fn uptime() -> Json<serde_json::Value> {
    Json(serde_json::json!(load_uptime_summary().unwrap_or_else(
        |_| burd_bench::UptimeSummary {
            uptime_1d: 0.0,
            uptime_7d: 0.0,
            uptime_30d: 0.0,
            last_online_at: None,
            last_failed_check_at: None,
            checks_total: 0,
            checks_failed: 0,
            current_status: "unknown".to_string(),
        }
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
    let message = match challenge_response_message(
        &challenge.challenge_id,
        &challenge.nonce,
        &config.provider_id,
        &config.machine_id,
        &signed_report.report_hash,
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
    let mut response = ChallengeResponse {
        challenge_id: challenge.challenge_id.clone(),
        nonce: challenge.nonce.clone(),
        provider_id: config.provider_id,
        machine_id: config.machine_id,
        report_hash: signed_report.report_hash.clone(),
        signed_report: Some(signed_report.clone()),
        signature,
        public_key: signed_report.public_key.clone(),
        completed_at: chrono::Utc::now().to_rfc3339(),
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
    let config = load_identity().ok();
    let auth_enabled = config
        .as_ref()
        .map(|config| config.api_auth_enabled && config.api_token_hash.is_some())
        .unwrap_or(false);

    if !auth_enabled {
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
    let token_configured = load_identity()
        .map(|config| config.api_auth_enabled && config.api_token_hash.is_some())
        .unwrap_or(false);
    if token_configured {
        None
    } else if host == "127.0.0.1" || host == "::1" || host == "localhost" {
        Some("dev mode: local API token is not configured on loopback".to_string())
    } else {
        Some(
            "strong warning: local API token is not configured while binding beyond loopback"
                .to_string(),
        )
    }
}

fn system_and_score(agent_version: &str) -> (burd_hardware::SystemReport, burd_bench::ScoreReport) {
    let specs = detect_specs();
    let system = detect_system_report(agent_version);
    let fit = build_fit_report(&specs, Some(25));
    let score = calculate_score(&system, Some(&fit), None, None, None, None);
    (system, score)
}
