use burd_control_plane::{AppState, ControlPlaneConfig, Database, router};
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        log_json("error", serde_json::json!({ "error": error }));
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let config = ControlPlaneConfig::from_env().map_err(|error| error.to_string())?;
    let db = Database::new(config.database_url.clone(), config.database_schema.clone())
        .map_err(|error| error.to_string())?;
    db.migrate().await.map_err(|error| error.to_string())?;

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .map_err(|error| format!("invalid bind address: {error}"))?;
    let state = Arc::new(AppState::new(config.clone(), db));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| format!("failed to bind {addr}: {error}"))?;

    log_json(
        "start",
        serde_json::json!({
            "service": "burd-control-plane",
            "host": config.host,
            "port": config.port,
            "environment": config.environment,
        }),
    );

    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|error| format!("server error: {error}"))
}

fn log_json(event: &str, fields: serde_json::Value) {
    eprintln!(
        "{}",
        serde_json::json!({
            "event": event,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "fields": fields,
        })
    );
}
