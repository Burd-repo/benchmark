use axum::Router;
use burd_control_plane::{AppState, ControlPlaneConfig, Database, router};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio_postgres::{Client, NoTls};
use uuid::Uuid;

struct RunningHttpServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl RunningHttpServer {
    async fn start(app: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });
        Self {
            addr,
            shutdown: Some(shutdown_tx),
            task,
        }
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await.unwrap();
    }
}

struct TestTcpProxy {
    addr: SocketAddr,
    drop_generation: watch::Sender<u64>,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl TestTcpProxy {
    async fn start(target: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Self::from_listener(listener, target)
    }

    async fn start_on(addr: SocketAddr, target: SocketAddr) -> Self {
        let listener = TcpListener::bind(addr).await.unwrap();
        Self::from_listener(listener, target)
    }

    fn from_listener(listener: TcpListener, target: SocketAddr) -> Self {
        let addr = listener.local_addr().unwrap();
        let (drop_generation, _) = watch::channel(0_u64);
        let task_drop_generation = drop_generation.clone();
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((downstream, _)) = accepted else {
                            break;
                        };
                        let drop_rx = task_drop_generation.subscribe();
                        connections.spawn(relay_connection(downstream, target, drop_rx));
                    }
                    completed = connections.join_next(), if !connections.is_empty() => {
                        let _ = completed;
                    }
                }
            }
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        });
        Self {
            addr,
            drop_generation,
            shutdown: Some(shutdown_tx),
            task,
        }
    }

    fn drop_connections(&self) {
        self.drop_generation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await.unwrap();
    }
}

async fn relay_connection(
    mut downstream: TcpStream,
    target: SocketAddr,
    mut drop_rx: watch::Receiver<u64>,
) {
    let Ok(mut upstream) = TcpStream::connect(target).await else {
        return;
    };
    let generation = *drop_rx.borrow();
    tokio::select! {
        result = copy_bidirectional(&mut downstream, &mut upstream) => {
            let _ = result;
        }
        _ = wait_for_proxy_drop(&mut drop_rx, generation) => {}
    }
}

async fn wait_for_proxy_drop(drop_rx: &mut watch::Receiver<u64>, generation: u64) {
    while *drop_rx.borrow() == generation {
        if drop_rx.changed().await.is_err() {
            return;
        }
    }
}

struct AgentStateGuard {
    previous_config: Option<OsString>,
    previous_home: Option<OsString>,
    state_dir: PathBuf,
}

impl AgentStateGuard {
    fn install(label: &str) -> Self {
        let state_dir = PathBuf::from(format!("target/test-agent-state/{label}"));
        std::fs::create_dir_all(&state_dir).unwrap();
        let config_path = state_dir.join("agent.json");
        let previous_config = std::env::var_os("BURD_AGENT_CONFIG");
        let previous_home = std::env::var_os("BURD_AGENT_HOME");
        // This ignored integration test serializes all process-wide Agent environment changes.
        unsafe {
            std::env::set_var("BURD_AGENT_CONFIG", &config_path);
            std::env::remove_var("BURD_AGENT_HOME");
        }
        Self {
            previous_config,
            previous_home,
            state_dir,
        }
    }
}

impl Drop for AgentStateGuard {
    fn drop(&mut self) {
        // Restore the process environment before another test can acquire the guard.
        unsafe {
            restore_env("BURD_AGENT_CONFIG", self.previous_config.take());
            restore_env("BURD_AGENT_HOME", self.previous_home.take());
        }
        let _ = std::fs::remove_dir_all(&self.state_dir);
    }
}

unsafe fn restore_env(name: &str, value: Option<OsString>) {
    match value {
        Some(value) => unsafe { std::env::set_var(name, value) },
        None => unsafe { std::env::remove_var(name) },
    }
}

fn agent_environment_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn post_json(url: String, payload: Value, headers: Vec<(String, String)>) -> (u16, Value) {
    tokio::task::spawn_blocking(move || {
        let mut request = ureq::post(&url)
            .config()
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .build();
        for (name, value) in &headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let mut response = request.send_json(payload).unwrap();
        let status = response.status().as_u16();
        let body = response.body_mut().read_json().unwrap_or(Value::Null);
        (status, body)
    })
    .await
    .unwrap()
}

async fn issue_enrollment_token(control_plane_url: &str) -> (String, String) {
    let (status, provider) = post_json(
        format!("{control_plane_url}/v1/providers"),
        json!({"display_name": "Agent Remote Session Harness"}),
        vec![
            ("Authorization".to_string(), "Bearer test-admin".to_string()),
            (
                "Idempotency-Key".to_string(),
                "agent-remote-session-harness".to_string(),
            ),
        ],
    )
    .await;
    assert_eq!(status, 201, "provider creation failed: {provider}");
    let provider_id = provider["provider"]["provider_id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, token) = post_json(
        format!("{control_plane_url}/v1/providers/{provider_id}/enrollment-tokens"),
        json!({}),
        vec![("Authorization".to_string(), "Bearer test-admin".to_string())],
    )
    .await;
    assert_eq!(status, 201, "enrollment token creation failed: {token}");
    (
        provider_id,
        token["enrollment_token"].as_str().unwrap().to_string(),
    )
}

#[derive(Debug, Clone)]
struct SessionRow {
    session_id: String,
    status: String,
    sequence_last: i64,
    resume_events: i64,
}

async fn session_rows(client: &Client) -> Vec<SessionRow> {
    client
        .query(
            "SELECT s.session_id, s.status, s.sequence_last, (SELECT COUNT(*)::BIGINT FROM audit_events a WHERE a.entity_type = 'provider_session' AND a.entity_id = s.session_id AND a.event_type = 'provider_session.resumed') AS resume_events FROM provider_sessions s ORDER BY s.started_at, s.session_id",
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| SessionRow {
            session_id: row.get("session_id"),
            status: row.get("status"),
            sequence_last: row.get("sequence_last"),
            resume_events: row.get("resume_events"),
        })
        .collect()
}

async fn wait_for_sessions<F>(
    client: &Client,
    description: &str,
    mut condition: F,
) -> Vec<SessionRow>
where
    F: FnMut(&[SessionRow]) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let rows = session_rows(client).await;
        if condition(&rows) {
            return rows;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {description}; last rows: {rows:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn schema_client(database_url: &str, schema: &str) -> Client {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .batch_execute(&format!("SET search_path TO {schema}"))
        .await
        .unwrap();
    client
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn live_agent_remote_session_reconnects_restarts_and_stops_on_revocation() {
    let _environment_lock = agent_environment_lock().lock().await;
    let database_url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
        .expect("BURD_CONTROL_TEST_DATABASE_URL is required for this ignored integration test");
    let label = format!("agent_remote_session_{}", Uuid::new_v4().simple());
    let schema = format!("burd_{label}");
    let object_storage_dir = PathBuf::from(format!("target/test-control-objects/{schema}"));
    let _agent_state = AgentStateGuard::install(&label);

    let mut config = ControlPlaneConfig::from_lookup(|key| match key {
        "BURD_CONTROL_DATABASE_URL" => Some(database_url.clone()),
        "BURD_CONTROL_ADMIN_TOKEN" => Some("test-admin".to_string()),
        _ => None,
    })
    .unwrap();
    config.environment = "test".to_string();
    config.database_schema = Some(schema.clone());
    config.object_storage_dir = object_storage_dir.display().to_string();
    config.rate_limit_per_minute = 1_000;
    config.remote_session_ttl_seconds = 8;
    config.heartbeat_interval_seconds = 1;
    config.missed_heartbeat_limit = 2;

    let db = Database::new(database_url.clone(), Some(schema.clone())).unwrap();
    db.migrate().await.unwrap();
    let app = router(Arc::new(AppState::new(config, db.clone())));
    let server = RunningHttpServer::start(app).await;
    let mut proxy = TestTcpProxy::start(server.addr).await;
    let control_plane_url = format!("http://{}", proxy.addr);

    let (expected_provider_id, enrollment_token) = issue_enrollment_token(&control_plane_url).await;
    tokio::task::spawn_blocking(burd_protocol::init_identity)
        .await
        .unwrap()
        .unwrap();
    let enrollment_url = control_plane_url.clone();
    let enrollment = tokio::task::spawn_blocking(move || {
        burd_agent::remote_enrollment::enroll(
            &enrollment_url,
            enrollment_token,
            "burd-agent-integration/0.1.0",
        )
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(enrollment.provider_id, expected_provider_id);

    let client = schema_client(&database_url, &schema).await;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut agent_task = tokio::spawn(async move {
        burd_agent::remote_session::connect_until_shutdown(
            "burd-agent-integration/0.1.0",
            1,
            false,
            4,
            shutdown_rx,
        )
        .await
    });

    let initial_rows =
        wait_for_sessions(&client, "the first acknowledged Agent heartbeat", |rows| {
            rows.len() == 1 && rows[0].status == "online" && rows[0].sequence_last >= 1
        })
        .await;
    let initial = initial_rows[0].clone();

    proxy.drop_connections();
    let resumed_rows =
        wait_for_sessions(&client, "the Agent to resume after socket loss", |rows| {
            rows.iter().any(|row| {
                row.session_id == initial.session_id
                    && row.status == "online"
                    && row.sequence_last > initial.sequence_last
                    && row.resume_events >= 1
            })
        })
        .await;
    let resumed = resumed_rows
        .iter()
        .find(|row| row.session_id == initial.session_id)
        .unwrap()
        .clone();
    assert!(resumed.sequence_last > initial.sequence_last);

    let proxy_addr = proxy.addr;
    proxy.stop().await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    proxy = TestTcpProxy::start_on(proxy_addr, server.addr).await;

    let restarted_rows = wait_for_sessions(
        &client,
        "an expired session replacement after the control plane returns",
        |rows| {
            rows.iter()
                .any(|row| row.session_id == initial.session_id && row.status == "expired")
                && rows.iter().any(|row| {
                    row.session_id != initial.session_id
                        && row.status == "online"
                        && row.sequence_last >= 1
                })
        },
    )
    .await;
    let restarted = restarted_rows
        .iter()
        .find(|row| row.session_id != initial.session_id && row.status == "online")
        .unwrap()
        .clone();
    assert_ne!(restarted.session_id, initial.session_id);

    let (status, revoked) = post_json(
        format!(
            "{control_plane_url}/v1/sessions/{}/revoke",
            restarted.session_id
        ),
        json!({}),
        vec![("Authorization".to_string(), "Bearer test-admin".to_string())],
    )
    .await;
    assert_eq!(status, 200, "session revocation failed: {revoked}");
    assert_eq!(revoked["status"], "revoked");

    let (timed_out, agent_result) =
        match tokio::time::timeout(Duration::from_secs(10), &mut agent_task).await {
            Ok(joined) => (false, joined.unwrap()),
            Err(_) => {
                let _ = shutdown_tx.send(true);
                let joined = tokio::time::timeout(Duration::from_secs(5), agent_task)
                    .await
                    .expect("Agent did not stop after integration-test shutdown")
                    .unwrap();
                (true, joined)
            }
        };

    proxy.stop().await;
    server.stop().await;
    db.drop_schema_for_test().await.unwrap();
    let _ = std::fs::remove_dir_all(&object_storage_dir);

    assert!(!timed_out, "Agent ignored remote session revocation");
    let error = agent_result.expect_err("revocation must terminate the Agent control loop");
    assert!(
        error.contains("revoked"),
        "unexpected Agent revocation error: {error}"
    );
}
