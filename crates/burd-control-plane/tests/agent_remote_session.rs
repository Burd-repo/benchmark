use axum::Router;
use burd_agent::remote_proof::{ProofExecution, ProofExecutionRequest};
use burd_control_plane::{AppState, ControlPlaneConfig, Database, router};
use burd_hardware::NvidiaTelemetryCollection;
use burd_protocol::{
    GpuTelemetrySample, ProofCapabilityMetrics, SignedProofCapabilityResponse,
    TelemetryBatchPayload,
};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio_postgres::{Client, NoTls};
use uuid::Uuid;

const TELEMETRY_MODE_VALID: u8 = 0;
const TELEMETRY_MODE_UNAVAILABLE: u8 = 1;
const TELEMETRY_MODE_INVALID: u8 = 2;

static TELEMETRY_MODE: AtomicU8 = AtomicU8::new(TELEMETRY_MODE_VALID);
static TELEMETRY_UNAVAILABLE_CALLS: AtomicU64 = AtomicU64::new(0);
static TELEMETRY_INVALID_CALLS: AtomicU64 = AtomicU64::new(0);

fn deterministic_nvidia_telemetry(
    first_sample_sequence: u64,
) -> Result<NvidiaTelemetryCollection, String> {
    let mode = TELEMETRY_MODE.load(Ordering::SeqCst);
    if mode == TELEMETRY_MODE_UNAVAILABLE {
        TELEMETRY_UNAVAILABLE_CALLS.fetch_add(1, Ordering::SeqCst);
        return Err("deterministic NVIDIA source paused by integration test".to_string());
    }

    let invalid = mode == TELEMETRY_MODE_INVALID;
    if invalid {
        TELEMETRY_INVALID_CALLS.fetch_add(1, Ordering::SeqCst);
    }
    let observed_at = chrono::Utc::now().to_rfc3339();
    Ok(NvidiaTelemetryCollection {
        collector: "burd-agent-integration-nvidia-v1".to_string(),
        samples: vec![GpuTelemetrySample {
            sample_sequence: first_sample_sequence,
            observed_at,
            gpu_uuid: "GPU-agent-integration".to_string(),
            gpu_name: "NVIDIA Integration GPU".to_string(),
            pci_bus_id: "00000000:01:00.0".to_string(),
            pci_vendor_id: Some("10de".to_string()),
            pci_device_id: Some("2684".to_string()),
            compute_capability: Some("8.9".to_string()),
            driver_version: "555.42".to_string(),
            cuda_driver_version: Some("12.5".to_string()),
            cuda_runtime_version: Some("12.4".to_string()),
            vram_total_mib: 24_564,
            vram_used_mib: Some(2_048),
            vram_free_mib: Some(22_516),
            gpu_utilization_percent: Some(if invalid { 101.0 } else { 42.0 }),
            memory_utilization_percent: Some(18.0),
            temperature_celsius: Some(58.0),
            power_draw_watts: Some(180.0),
            power_limit_watts: Some(320.0),
            graphics_clock_mhz: Some(1_800),
            sm_clock_mhz: Some(1_800),
            memory_clock_mhz: Some(10_500),
            performance_state: Some("P2".to_string()),
            throttle_reasons: vec![],
            ecc_corrected_errors: None,
            ecc_uncorrected_errors: None,
            processes: vec![],
            container_id: None,
            job_id: None,
        }],
        warnings: vec![],
    })
}
fn deterministic_proof_telemetry(
    first_sample_sequence: u64,
) -> Result<NvidiaTelemetryCollection, String> {
    let mut collection = deterministic_nvidia_telemetry(first_sample_sequence)?;
    collection.samples[0].gpu_utilization_percent = Some(5.0);
    Ok(collection)
}
fn deterministic_proof_executor(
    mut request: ProofExecutionRequest,
) -> Result<ProofExecution, String> {
    let gpu_uuid = request
        .challenge
        .required_gpu_uuid
        .clone()
        .unwrap_or_else(|| "GPU-agent-integration".to_string());
    request.hold_residency_for_telemetry(gpu_uuid.clone())?;
    Ok(ProofExecution {
        gpu_uuid,
        driver_version: "555.42".to_string(),
        cuda_driver_version: Some("12.5".to_string()),
        cuda_runtime_version: Some("12.4".to_string()),
        metrics: ProofCapabilityMetrics {
            tokens_per_second: Some(72.5),
            ttft_ms: Some(85),
            vram_allocated_mib: Some(64),
            vram_resident_mib: Some(64),
            gemm_gflops: Some(21_000.0),
            cuda_runtime_detected: true,
            backend_proof: "integration-test-only-deterministic-proof-v1".to_string(),
            contention_detected: false,
        },
    })
}
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

#[derive(Debug, Clone)]
struct TelemetryBatchRow {
    control_sequence: u64,
    sample_sequence_start: u64,
    sample_sequence_end: u64,
    sample_count: u32,
    persisted_sample_count: u64,
    batch_hash: String,
    public_key_id: String,
    signature: String,
    canonicalization_version: String,
    payload_json: String,
    verification_json: String,
    public_key: String,
}

async fn telemetry_batch_rows(client: &Client, session_id: &str) -> Vec<TelemetryBatchRow> {
    client
        .query(
            "SELECT t.control_sequence, t.sample_sequence_start, t.sample_sequence_end, t.sample_count, t.batch_hash, t.public_key_id, t.signature, t.canonicalization_version, t.payload_json, t.verification_json, k.public_key, (SELECT COUNT(*)::BIGINT FROM gpu_telemetry_samples s WHERE s.batch_id = t.batch_id) AS persisted_sample_count FROM telemetry_batches t JOIN provider_public_keys k ON k.public_key_id = t.public_key_id WHERE t.session_id = $1 ORDER BY t.sample_sequence_start",
            &[&session_id],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| TelemetryBatchRow {
            control_sequence: row.get::<_, i64>("control_sequence").max(0) as u64,
            sample_sequence_start: row.get::<_, i64>("sample_sequence_start").max(0) as u64,
            sample_sequence_end: row.get::<_, i64>("sample_sequence_end").max(0) as u64,
            sample_count: row.get::<_, i32>("sample_count").max(0) as u32,
            persisted_sample_count: row.get::<_, i64>("persisted_sample_count").max(0) as u64,
            batch_hash: row.get("batch_hash"),
            public_key_id: row.get("public_key_id"),
            signature: row.get("signature"),
            canonicalization_version: row.get("canonicalization_version"),
            payload_json: row.get("payload_json"),
            verification_json: row.get("verification_json"),
            public_key: row.get("public_key"),
        })
        .collect()
}

async fn wait_for_telemetry_batches<F>(
    client: &Client,
    session_id: &str,
    description: &str,
    mut condition: F,
) -> Vec<TelemetryBatchRow>
where
    F: FnMut(&[TelemetryBatchRow]) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let rows = telemetry_batch_rows(client, session_id).await;
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

#[derive(Debug, Clone)]
struct ProofChallengeRow {
    status: String,
    response_hash: Option<String>,
    response_json: Option<String>,
    verification_json: Option<String>,
    public_key: Option<String>,
}

async fn proof_challenge_row(client: &Client, challenge_id: &str) -> ProofChallengeRow {
    let row = client
        .query_one(
            "SELECT pc.status, pc.response_hash, pc.response_json, pc.verification_json, k.public_key FROM proof_challenges pc LEFT JOIN provider_public_keys k ON k.public_key_id = pc.public_key_id WHERE pc.challenge_id = $1",
            &[&challenge_id],
        )
        .await
        .unwrap();
    ProofChallengeRow {
        status: row.get("status"),
        response_hash: row.get("response_hash"),
        response_json: row.get("response_json"),
        verification_json: row.get("verification_json"),
        public_key: row.get("public_key"),
    }
}

async fn wait_for_verified_proof(client: &Client, challenge_id: &str) -> ProofChallengeRow {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    loop {
        let row = proof_challenge_row(client, challenge_id).await;
        if row.status == "verified" {
            return row;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for Agent proof {challenge_id}; last row: {row:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn wait_for_counter(counter: &AtomicU64, expected: u64, description: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while counter.load(Ordering::SeqCst) < expected {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {description}; current count: {}",
            counter.load(Ordering::SeqCst)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_local_telemetry_sequence(expected: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = tokio::task::spawn_blocking(burd_protocol::load_remote_session)
            .await
            .unwrap()
            .unwrap();
        if state.telemetry_sequence_last >= expected {
            assert_eq!(state.telemetry_sequence_last, expected);
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for local telemetry ACK sequence {expected}; current sequence: {}",
            state.telemetry_sequence_last
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn live_agent_signed_telemetry_persists_ack_resumes_and_handles_rejection() {
    let _environment_lock = agent_environment_lock().lock().await;
    TELEMETRY_MODE.store(TELEMETRY_MODE_VALID, Ordering::SeqCst);
    TELEMETRY_UNAVAILABLE_CALLS.store(0, Ordering::SeqCst);
    TELEMETRY_INVALID_CALLS.store(0, Ordering::SeqCst);

    let database_url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
        .expect("BURD_CONTROL_TEST_DATABASE_URL is required for this ignored integration test");
    let label = format!("agent_signed_telemetry_{}", Uuid::new_v4().simple());
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
    config.remote_session_ttl_seconds = 30;
    config.heartbeat_interval_seconds = 1;
    config.missed_heartbeat_limit = 2;
    config.telemetry_min_batch_interval_seconds = 0;

    let db = Database::new(database_url.clone(), Some(schema.clone())).unwrap();
    db.migrate().await.unwrap();
    let app = router(Arc::new(AppState::new(config, db.clone())));
    let server = RunningHttpServer::start(app).await;
    let proxy = TestTcpProxy::start(server.addr).await;
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
        burd_agent::remote_session::connect_until_shutdown_with_telemetry_collector(
            "burd-agent-integration/0.1.0",
            1,
            true,
            1,
            deterministic_nvidia_telemetry,
            shutdown_rx,
        )
        .await
    });

    let initial_sessions = wait_for_sessions(
        &client,
        "the telemetry Agent session to become online",
        |rows| rows.len() == 1 && rows[0].status == "online" && rows[0].sequence_last >= 1,
    )
    .await;
    let initial_session = initial_sessions[0].clone();
    let accepted = wait_for_telemetry_batches(
        &client,
        &initial_session.session_id,
        "the Agent-produced signed telemetry batch",
        |rows| !rows.is_empty(),
    )
    .await;
    let first = &accepted[0];
    assert_eq!(first.sample_sequence_start, 1);
    assert_eq!(first.sample_sequence_end, 1);
    assert_eq!(first.sample_count, 1);
    assert_eq!(first.persisted_sample_count, 1);
    assert_eq!(
        first.canonicalization_version,
        burd_protocol::TELEMETRY_CANONICALIZATION_VERSION
    );

    let payload: TelemetryBatchPayload = serde_json::from_str(&first.payload_json).unwrap();
    assert_eq!(payload.provider_id, expected_provider_id);
    assert_eq!(payload.device_id, enrollment.device_id);
    assert_eq!(payload.session_id, initial_session.session_id);
    assert_eq!(payload.control_sequence, first.control_sequence);
    assert_eq!(payload.collector, "burd-agent-integration-nvidia-v1");
    assert_eq!(payload.samples[0].gpu_uuid, "GPU-agent-integration");
    assert_eq!(payload.samples[0].gpu_utilization_percent, Some(42.0));
    assert_eq!(
        burd_protocol::telemetry_batch_hash(&payload).unwrap(),
        first.batch_hash
    );
    let signature_message = burd_protocol::telemetry_batch_signature_message(
        &payload,
        &first.batch_hash,
        &first.public_key_id,
    )
    .unwrap();
    assert!(
        burd_protocol::verify_message(
            &first.public_key,
            signature_message.as_bytes(),
            &first.signature,
        )
        .unwrap()
    );
    let verification: Value = serde_json::from_str(&first.verification_json).unwrap();
    assert_eq!(verification["hash_valid"], true);
    assert_eq!(verification["signature_valid"], true);
    assert_eq!(verification["session_bound"], true);
    assert_eq!(verification["fingerprint_bound"], true);

    TELEMETRY_MODE.store(TELEMETRY_MODE_UNAVAILABLE, Ordering::SeqCst);
    wait_for_counter(
        &TELEMETRY_UNAVAILABLE_CALLS,
        1,
        "the deterministic source to pause before reconnect",
    )
    .await;
    let before_drop = telemetry_batch_rows(&client, &initial_session.session_id).await;
    let sequence_before_drop = before_drop.last().unwrap().sample_sequence_end;
    wait_for_local_telemetry_sequence(sequence_before_drop).await;

    proxy.drop_connections();
    wait_for_sessions(&client, "the telemetry Agent session to resume", |rows| {
        rows.iter().any(|row| {
            row.session_id == initial_session.session_id
                && row.status == "online"
                && row.resume_events >= 1
        })
    })
    .await;

    TELEMETRY_MODE.store(TELEMETRY_MODE_VALID, Ordering::SeqCst);
    let after_resume = wait_for_telemetry_batches(
        &client,
        &initial_session.session_id,
        "telemetry sequence continuation after reconnect",
        |rows| {
            rows.last()
                .is_some_and(|row| row.sample_sequence_end > sequence_before_drop)
        },
    )
    .await;
    TELEMETRY_MODE.store(TELEMETRY_MODE_UNAVAILABLE, Ordering::SeqCst);
    let unavailable_before = TELEMETRY_UNAVAILABLE_CALLS.load(Ordering::SeqCst);
    wait_for_counter(
        &TELEMETRY_UNAVAILABLE_CALLS,
        unavailable_before + 1,
        "the deterministic source to pause after reconnect",
    )
    .await;

    let accepted_after_resume = telemetry_batch_rows(&client, &initial_session.session_id).await;
    assert!(accepted_after_resume.len() >= after_resume.len());
    for (offset, row) in accepted_after_resume.iter().enumerate() {
        let expected_sequence = offset as u64 + 1;
        assert_eq!(row.sample_sequence_start, expected_sequence);
        assert_eq!(row.sample_sequence_end, expected_sequence);
        assert_eq!(row.sample_count, 1);
        assert_eq!(row.persisted_sample_count, 1);
    }
    let last_accepted = accepted_after_resume.last().unwrap();
    let accepted_count = accepted_after_resume.len();
    let accepted_control_sequence = last_accepted.control_sequence;
    let accepted_sample_sequence = last_accepted.sample_sequence_end;
    wait_for_local_telemetry_sequence(accepted_sample_sequence).await;

    let sequence_before_rejection = session_rows(&client)
        .await
        .into_iter()
        .find(|row| row.session_id == initial_session.session_id)
        .unwrap()
        .sequence_last;
    TELEMETRY_MODE.store(TELEMETRY_MODE_INVALID, Ordering::SeqCst);
    wait_for_counter(
        &TELEMETRY_INVALID_CALLS,
        1,
        "the Agent to submit invalid deterministic telemetry",
    )
    .await;
    wait_for_sessions(
        &client,
        "heartbeats to continue after telemetry rejection",
        |rows| {
            rows.iter().any(|row| {
                row.session_id == initial_session.session_id
                    && row.status == "online"
                    && row.sequence_last > sequence_before_rejection
                    && row.sequence_last > i64::try_from(accepted_control_sequence).unwrap()
            })
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(2_200)).await;

    assert_eq!(
        TELEMETRY_INVALID_CALLS.load(Ordering::SeqCst),
        1,
        "the Agent must disable telemetry after a server rejection"
    );
    assert_eq!(
        telemetry_batch_rows(&client, &initial_session.session_id)
            .await
            .len(),
        accepted_count,
        "rejected telemetry must not be persisted"
    );
    assert!(
        !agent_task.is_finished(),
        "telemetry rejection must not terminate the remote session"
    );
    wait_for_local_telemetry_sequence(accepted_sample_sequence).await;

    let _ = shutdown_tx.send(true);
    let agent_result = tokio::time::timeout(Duration::from_secs(5), &mut agent_task)
        .await
        .expect("Agent did not stop after integration-test shutdown")
        .unwrap();
    assert!(
        agent_result.is_ok(),
        "unexpected Agent error: {agent_result:?}"
    );

    proxy.stop().await;
    server.stop().await;
    db.drop_schema_for_test().await.unwrap();
    let _ = std::fs::remove_dir_all(&object_storage_dir);
    TELEMETRY_MODE.store(TELEMETRY_MODE_VALID, Ordering::SeqCst);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn live_agent_executes_and_submits_a_verified_remote_proof() {
    let _environment_lock = agent_environment_lock().lock().await;
    TELEMETRY_MODE.store(TELEMETRY_MODE_VALID, Ordering::SeqCst);

    let database_url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
        .expect("BURD_CONTROL_TEST_DATABASE_URL is required for this ignored integration test");
    let label = format!("agent_remote_proof_{}", Uuid::new_v4().simple());
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
    config.remote_session_ttl_seconds = 30;
    config.heartbeat_interval_seconds = 1;
    config.missed_heartbeat_limit = 2;
    config.telemetry_min_batch_interval_seconds = 0;

    let db = Database::new(database_url.clone(), Some(schema.clone())).unwrap();
    db.migrate().await.unwrap();
    let app = router(Arc::new(AppState::new(config, db.clone())));
    let server = RunningHttpServer::start(app).await;
    let control_plane_url = format!("http://{}", server.addr);

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
            "burd-agent-proof-integration/0.1.0",
        )
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(enrollment.provider_id, expected_provider_id);

    let client = schema_client(&database_url, &schema).await;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut agent_task = tokio::spawn(async move {
        burd_agent::remote_session::connect_until_shutdown_with_test_runtime(
            "burd-agent-proof-integration/0.1.0",
            1,
            8,
            deterministic_proof_telemetry,
            deterministic_proof_executor,
            shutdown_rx,
        )
        .await
    });

    let sessions = wait_for_sessions(
        &client,
        "the proof Agent session to become online",
        |rows| rows.len() == 1 && rows[0].status == "online" && rows[0].sequence_last >= 1,
    )
    .await;
    let session = sessions[0].clone();

    let fingerprint: String = client
        .query_one(
            "SELECT hardware_fingerprint FROM provider_sessions WHERE session_id = $1",
            &[&session.session_id],
        )
        .await
        .unwrap()
        .get("hardware_fingerprint");

    let (status, issued) = post_json(
        format!("{control_plane_url}/v1/challenges"),
        json!({
            "provider_id": expected_provider_id,
            "device_id": enrollment.device_id,
            "session_id": session.session_id,
            "profile_version": "burd-poc-integration-v1",
            "required_fingerprint": fingerprint,
            "required_gpu_uuid": "GPU-agent-integration",
            "required_backend": "cuda",
            "model_artifact_hash": "sha256:agent-proof-integration-model",
            "prompt_seed": "agent-proof-integration-seed",
            "required_proofs": [
                "cuda_runtime",
                "vram_allocation_residency",
                "tensor_gemm_microbenchmark",
                "llm_short_inference",
                "performance_consistency",
                "contention_detection",
                "telemetry_window"
            ],
            "min_tokens_per_second": 10.0,
            "max_ttft_ms": 500,
            "expires_in_seconds": 120
        }),
        vec![("Authorization".to_string(), "Bearer test-admin".to_string())],
    )
    .await;
    assert_eq!(status, 201, "proof challenge issue failed: {issued}");
    let challenge_id = issued["challenge"]["challenge_id"]
        .as_str()
        .unwrap()
        .to_string();
    let challenge_nonce = issued["challenge"]["nonce"].as_str().unwrap().to_string();

    let row = wait_for_verified_proof(&client, &challenge_id).await;
    let response: SignedProofCapabilityResponse =
        serde_json::from_str(row.response_json.as_deref().unwrap()).unwrap();
    assert_eq!(response.payload.challenge_id, challenge_id);
    assert_eq!(response.payload.nonce, challenge_nonce);
    assert_eq!(response.payload.provider_id, expected_provider_id);
    assert_eq!(response.payload.device_id, enrollment.device_id);
    assert_eq!(response.payload.session_id, session.session_id);
    assert_eq!(response.payload.hardware_fingerprint, fingerprint);
    assert_eq!(response.payload.gpu_uuid, "GPU-agent-integration");
    assert_eq!(
        response.payload.metrics.backend_proof,
        "integration-test-only-deterministic-proof-v1"
    );
    assert_eq!(
        burd_protocol::proof_capability_response_hash(&response.payload).unwrap(),
        response.response_hash
    );
    assert_eq!(
        row.response_hash.as_deref(),
        Some(response.response_hash.as_str())
    );
    let signature_message = burd_protocol::proof_capability_response_signature_message(
        &response.payload,
        &response.response_hash,
        &response.public_key_id,
    )
    .unwrap();
    assert!(
        burd_protocol::verify_message(
            row.public_key.as_deref().unwrap(),
            signature_message.as_bytes(),
            &response.signature,
        )
        .unwrap()
    );

    let verification: Value =
        serde_json::from_str(row.verification_json.as_deref().unwrap()).unwrap();
    for field in [
        "response_hash_valid",
        "signature_valid",
        "provider_bound",
        "device_bound",
        "session_bound",
        "fingerprint_bound",
        "gpu_bound",
        "backend_bound",
        "artifact_bound",
        "prompt_bound",
        "metrics_satisfied",
    ] {
        assert_eq!(
            verification[field], true,
            "verification field {field} failed"
        );
    }
    assert_eq!(verification["expired_by_server"], false);
    assert_eq!(verification["errors"], json!([]));
    let telemetry_window_hash = response.payload.telemetry_window_hash.as_deref().unwrap();
    let telemetry_link_exists: bool = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM telemetry_batches WHERE batch_hash = $1 AND session_id = $2)",
            &[&telemetry_window_hash, &session.session_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(telemetry_link_exists);
    let linked_sample_count: i64 = client
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM gpu_telemetry_samples sample JOIN telemetry_batches batch ON batch.batch_id = sample.batch_id WHERE batch.batch_hash = $1",
            &[&telemetry_window_hash],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        linked_sample_count, 1,
        "proof telemetry must contain only the in-window forced sample"
    );

    let _ = shutdown_tx.send(true);
    let agent_result = tokio::time::timeout(Duration::from_secs(5), &mut agent_task)
        .await
        .expect("Agent did not stop after remote proof integration-test shutdown")
        .unwrap();
    assert!(
        agent_result.is_ok(),
        "unexpected Agent error: {agent_result:?}"
    );

    server.stop().await;
    db.drop_schema_for_test().await.unwrap();
    let _ = std::fs::remove_dir_all(&object_storage_dir);
}
