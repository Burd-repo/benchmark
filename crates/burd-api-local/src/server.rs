use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use burd_bench::{ReportRunOptions, calculate_score, generate_full_report};
use burd_hardware::{detect_specs, detect_system_report};
use burd_llmfit::build_fit_report;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::RwLock;

static UI_INDEX: &str = include_str!("../../../apps/benchmark-ui/index.html");
static UI_STYLES: &str = include_str!("../../../apps/benchmark-ui/styles.css");

#[derive(Debug)]
struct AppState {
    agent_version: String,
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
        .route("/api/v1/score", get(score))
        .route("/api/v1/report", get(report))
        .route("/api/v1/benchmark/run", post(run_benchmark))
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
    }))
}

async fn system(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(detect_system_report(
        &state.agent_version
    )))
}

async fn score(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let specs = detect_specs();
    let system = detect_system_report(&state.agent_version);
    let fit = build_fit_report(&specs, Some(25));
    let score = calculate_score(&system, Some(&fit), None, None, None, None);
    Json(serde_json::json!(score))
}

async fn report(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let report = generate_full_report(ReportRunOptions::new(state.agent_version.clone()));
    Json(serde_json::json!(report))
}

async fn run_benchmark(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    {
        let mut status = state.benchmark_status.write().await;
        status.status = "running".to_string();
        status.last_report = None;
    }

    let mut options = ReportRunOptions::new(state.agent_version.clone());
    options.run_all = true;
    let report = generate_full_report(options);
    let report_json = serde_json::json!(report);

    {
        let mut status = state.benchmark_status.write().await;
        status.status = "completed".to_string();
        status.last_report = Some(report_json.clone());
    }

    Json(serde_json::json!({
        "status": "completed",
        "report": report_json,
    }))
}

async fn benchmark_status(State(state): State<Arc<AppState>>) -> Json<BenchmarkStatus> {
    Json(state.benchmark_status.read().await.clone())
}
