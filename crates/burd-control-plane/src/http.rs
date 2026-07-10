use crate::config::ControlPlaneConfig;
use crate::db::{CreateProviderCommand, CreateProviderOutcome, Database, ProviderRecord};
use crate::enrollment::EnrollmentError;
use crate::error::{ApiError, ErrorCode};
use crate::openapi;
use crate::proof_challenge::ProofChallengePolicy;
use crate::rate_limit::RateLimiter;
use crate::remote_session::{
    AuthorizedSession, ControlChannelLease, ControlChannelRegistry, RemoteSessionPolicy,
    SessionError,
};
use crate::telemetry::TelemetryPolicy;
use crate::verification_policy::VerificationPolicy;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use burd_protocol::{
    ClientControlMessage, EnrollmentProofRequest, IssueProofChallengeRequest,
    KeyRotationProofRequest, RevokeEvidenceRequest, RunVerificationSweepRequest,
    ServerControlMessage, SignedProofCapabilityResponse, StartEnrollmentRequest,
    StartKeyRotationRequest, StartRemoteSessionRequest, SubmitEvidenceRequest, hash_canonical,
    sha256_hex,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug)]
pub struct AppState {
    pub config: ControlPlaneConfig,
    pub db: Database,
    pub rate_limiter: RateLimiter,
    pub control_channels: ControlChannelRegistry,
}

impl AppState {
    pub fn new(config: ControlPlaneConfig, db: Database) -> Self {
        let rate_limiter = RateLimiter::per_minute(config.rate_limit_per_minute);
        Self {
            config,
            db,
            rate_limiter,
            control_channels: ControlChannelRegistry::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
    environment: String,
    request_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct ReadyResponse {
    status: &'static str,
    service: &'static str,
    database: &'static str,
    migrations_applied: Vec<String>,
    migrations_expected: Vec<&'static str>,
    request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProviderRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct EvidenceListQuery {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderEnvelope {
    request_id: String,
    audit_event_id: Option<String>,
    provider: ProviderRecord,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/openapi.json", get(openapi_json))
        .route("/v1/providers", post(create_provider))
        .route("/v1/providers/{provider_id}", get(get_provider))
        .route(
            "/v1/providers/{provider_id}/enrollment-tokens",
            post(issue_enrollment_token),
        )
        .route(
            "/v1/providers/{provider_id}/devices",
            get(list_provider_devices),
        )
        .route("/v1/enrollments", post(start_enrollment))
        .route(
            "/v1/enrollments/{enrollment_id}/proof",
            post(complete_enrollment),
        )
        .route(
            "/v1/devices/{device_id}/credentials",
            post(refresh_device_credential),
        )
        .route(
            "/v1/devices/{device_id}/key-rotations",
            post(start_key_rotation),
        )
        .route(
            "/v1/devices/{device_id}/key-rotations/{rotation_id}/proof",
            post(complete_key_rotation),
        )
        .route("/v1/devices/{device_id}/revoke", post(revoke_device))
        .route("/v1/sessions", post(start_remote_session))
        .route("/v1/sessions/{session_id}", get(get_remote_session))
        .route(
            "/v1/sessions/{session_id}/heartbeats",
            post(record_remote_heartbeat),
        )
        .route(
            "/v1/sessions/{session_id}/control",
            get(upgrade_control_channel),
        )
        .route(
            "/v1/sessions/{session_id}/revoke",
            post(revoke_remote_session),
        )
        .route(
            "/v1/sessions/{session_id}/telemetry-batches",
            post(ingest_gpu_telemetry),
        )
        .route(
            "/v1/sessions/{session_id}/telemetry/latest",
            get(latest_gpu_telemetry),
        )
        .route(
            "/v1/sessions/{session_id}/evidence-records",
            post(submit_evidence_record),
        )
        .route(
            "/v1/providers/{provider_id}/evidence-records",
            get(list_provider_evidence_records),
        )
        .route(
            "/v1/evidence-records/{evidence_id}",
            get(get_evidence_record),
        )
        .route(
            "/v1/evidence-records/{evidence_id}/revoke",
            post(revoke_evidence_record),
        )
        .route("/v1/verification/sweep", post(run_verification_sweep))
        .route(
            "/v1/providers/{provider_id}/verification-states",
            get(list_provider_verification_states),
        )
        .route("/v1/challenges", post(issue_proof_challenge))
        .route("/v1/challenges/{challenge_id}", get(get_proof_challenge))
        .route(
            "/v1/sessions/{session_id}/challenges/next",
            get(next_proof_challenge),
        )
        .route(
            "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
            post(submit_proof_challenge_response),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "burd-control-plane",
        version: env!("CARGO_PKG_VERSION"),
        environment: state.config.environment.clone(),
        request_id: new_request_id(),
    })
}

async fn ready(State(state): State<Arc<AppState>>) -> Result<Json<ReadyResponse>, ApiError> {
    let request_id = new_request_id();
    state
        .db
        .health_check()
        .await
        .map_err(|error| ApiError::database(error, request_id.clone()))?;
    let applied = state
        .db
        .migration_versions()
        .await
        .map_err(|error| ApiError::database(error, request_id.clone()))?;
    let expected = crate::migrations::MIGRATIONS
        .iter()
        .map(|migration| migration.version)
        .collect::<Vec<_>>();
    if !migrations_are_current(&applied, &expected) {
        return Err(ApiError::database(
            format!(
                "control plane migrations are incomplete; applied={applied:?}, expected={expected:?}"
            ),
            request_id,
        ));
    }

    Ok(Json(ReadyResponse {
        status: "ready",
        service: "burd-control-plane",
        database: "ok",
        migrations_applied: applied,
        migrations_expected: expected,
        request_id,
    }))
}

async fn openapi_json() -> Json<serde_json::Value> {
    Json(openapi::document())
}

async fn create_provider(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateProviderRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let request_hash = hash_canonical(&payload)
        .map_err(|error| ApiError::invalid_request(error, request_id.clone()))?;

    let outcome = state
        .db
        .create_provider_idempotently(CreateProviderCommand {
            request_id: request_id.clone(),
            scope: "POST /v1/providers".to_string(),
            idempotency_key,
            request_hash,
            user_id: payload.user_id,
            display_name: payload.display_name,
        })
        .await
        .map_err(|error| ApiError::database(error, request_id.clone()))?;

    match outcome {
        CreateProviderOutcome::Response(record) => {
            let value = serde_json::from_str::<serde_json::Value>(&record.response_json).map_err(
                |error| ApiError::invalid_request(error.to_string(), request_id.clone()),
            )?;
            let status = StatusCode::from_u16(record.status_code).unwrap_or(StatusCode::OK);
            Ok((status, Json(value)).into_response())
        }
        CreateProviderOutcome::Conflict => Err(ApiError::idempotency_conflict(request_id)),
    }
}

async fn get_provider(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = new_request_id();
    let provider = state
        .db
        .get_provider(&provider_id)
        .await
        .map_err(|error| ApiError::database(error, request_id.clone()))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "provider not found",
                request_id.clone(),
            )
        })?;

    Ok(Json(serde_json::json!(ProviderEnvelope {
        request_id,
        audit_event_id: None,
        provider,
    })))
}

async fn issue_enrollment_token(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .issue_enrollment_token(
            &provider_id,
            &request_id,
            state.config.enrollment_token_ttl_seconds,
        )
        .await
        .map_err(|error| enrollment_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn start_enrollment(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StartEnrollmentRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let response = state
        .db
        .start_enrollment(
            &request_id,
            &payload,
            state.config.enrollment_proof_ttl_seconds,
        )
        .await
        .map_err(|error| enrollment_api_error(error, request_id.clone()))?;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

async fn complete_enrollment(
    State(state): State<Arc<AppState>>,
    Path(enrollment_id): Path<String>,
    Json(payload): Json<EnrollmentProofRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let response = state
        .db
        .complete_enrollment(
            &enrollment_id,
            &request_id,
            &payload,
            state.config.device_credential_ttl_seconds,
        )
        .await
        .map_err(|error| enrollment_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn list_provider_devices(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let devices = state
        .db
        .list_provider_devices(&provider_id)
        .await
        .map_err(|error| enrollment_api_error(error, request_id.clone()))?;
    Ok(Json(serde_json::json!({
        "request_id": request_id,
        "devices": devices,
    })))
}

async fn refresh_device_credential(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let credential = required_bearer_token(&headers, &request_id)?;
    let response = state
        .db
        .refresh_device_credential(
            &device_id,
            &credential,
            &request_id,
            state.config.device_credential_ttl_seconds,
        )
        .await
        .map_err(|error| enrollment_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn start_key_rotation(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<StartKeyRotationRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let credential = required_bearer_token(&headers, &request_id)?;
    let response = state
        .db
        .start_key_rotation(
            &device_id,
            &credential,
            &request_id,
            &payload,
            state.config.enrollment_proof_ttl_seconds,
        )
        .await
        .map_err(|error| enrollment_api_error(error, request_id.clone()))?;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

async fn complete_key_rotation(
    State(state): State<Arc<AppState>>,
    Path((device_id, rotation_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<KeyRotationProofRequest>,
) -> Result<Json<burd_protocol::KeyRotationProofResponse>, ApiError> {
    let request_id = new_request_id();
    let credential = required_bearer_token(&headers, &request_id)?;
    let response = state
        .db
        .complete_key_rotation(&device_id, &rotation_id, &credential, &request_id, &payload)
        .await
        .map_err(|error| enrollment_api_error(error, request_id.clone()))?;
    Ok(Json(response))
}

async fn revoke_device(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::DeviceRevocationResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .revoke_device(&device_id, &request_id)
        .await
        .map_err(|error| enrollment_api_error(error, request_id.clone()))?;
    Ok(Json(response))
}

async fn start_remote_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<StartRemoteSessionRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let credential = required_bearer_token(&headers, &request_id)?;
    let response = state
        .db
        .start_remote_session(
            &request_id,
            &credential,
            &payload,
            RemoteSessionPolicy {
                ttl_seconds: state.config.remote_session_ttl_seconds,
                heartbeat_interval_seconds: state.config.heartbeat_interval_seconds,
                missed_heartbeat_limit: state.config.missed_heartbeat_limit,
            },
            control_channel_url(&headers),
        )
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn get_remote_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::RemoteSessionRecord>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, true).await?;
    state
        .db
        .get_remote_session(&session_id, &authorized, &request_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn record_remote_heartbeat(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(message): Json<ClientControlMessage>,
) -> Result<Json<burd_protocol::HeartbeatReceipt>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .record_remote_heartbeat(
            &request_id,
            &authorized,
            &message,
            state.config.remote_session_ttl_seconds,
        )
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn ingest_gpu_telemetry(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(message): Json<ClientControlMessage>,
) -> Result<Json<burd_protocol::TelemetryBatchReceipt>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .ingest_gpu_telemetry(
            &request_id,
            &authorized,
            &message,
            telemetry_policy(&state.config),
        )
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn latest_gpu_telemetry(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::LatestTelemetryResponse>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, true).await?;
    state
        .db
        .latest_gpu_telemetry(&request_id, &authorized)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn submit_evidence_record(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<SubmitEvidenceRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    let response = state
        .db
        .submit_evidence_record(
            &request_id,
            &authorized,
            &session_id,
            &payload,
            &state.config.object_storage_dir,
        )
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    let status = if response.duplicate {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(response)).into_response())
}

async fn list_provider_evidence_records(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<EvidenceListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListEvidenceResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_evidence_records(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn get_evidence_record(
    State(state): State<Arc<AppState>>,
    Path(evidence_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::EvidenceRecord>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let evidence = state
        .db
        .get_evidence_record(&evidence_id)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "evidence record not found",
                request_id.clone(),
            )
        })?;
    Ok(Json(evidence))
}

async fn revoke_evidence_record(
    State(state): State<Arc<AppState>>,
    Path(evidence_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<RevokeEvidenceRequest>,
) -> Result<Json<burd_protocol::RevokeEvidenceResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .revoke_evidence_record(&evidence_id, &request_id, &payload.reason)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
async fn run_verification_sweep(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<RunVerificationSweepRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .run_verification_sweep(
            &request_id,
            &payload,
            proof_challenge_policy(&state.config),
            verification_policy(&state.config),
        )
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

async fn list_provider_verification_states(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListVerificationStatesResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_verification_states(&request_id, &provider_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
async fn issue_proof_challenge(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<IssueProofChallengeRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .issue_proof_challenge(&request_id, &payload, proof_challenge_policy(&state.config))
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn get_proof_challenge(
    State(state): State<Arc<AppState>>,
    Path(challenge_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ProofChallengeRecord>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let record = state
        .db
        .get_proof_challenge(&challenge_id)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "proof challenge not found",
                request_id.clone(),
            )
        })?;
    Ok(Json(record))
}

async fn next_proof_challenge(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::NextProofChallengeResponse>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .next_proof_challenge(&request_id, &authorized)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn submit_proof_challenge_response(
    State(state): State<Arc<AppState>>,
    Path((session_id, challenge_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<SignedProofCapabilityResponse>,
) -> Result<Json<burd_protocol::SubmitProofChallengeResponse>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .submit_proof_challenge_response(
            &request_id,
            &authorized,
            &session_id,
            &challenge_id,
            &payload,
            &state.config.object_storage_dir,
            proof_challenge_policy(&state.config),
            verification_policy(&state.config),
        )
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
fn telemetry_policy(config: &ControlPlaneConfig) -> TelemetryPolicy {
    TelemetryPolicy {
        max_samples_per_batch: config.telemetry_max_samples_per_batch,
        min_batch_interval_seconds: config.telemetry_min_batch_interval_seconds,
        clock_skew_seconds: config.telemetry_clock_skew_seconds,
    }
}
fn proof_challenge_policy(config: &ControlPlaneConfig) -> ProofChallengePolicy {
    ProofChallengePolicy {
        ttl_seconds: config.proof_challenge_ttl_seconds,
        clock_skew_seconds: config.proof_challenge_clock_skew_seconds,
    }
}
fn verification_policy(config: &ControlPlaneConfig) -> VerificationPolicy {
    VerificationPolicy {
        period_seconds: config.verification_period_seconds,
        retry_budget: config.verification_retry_budget,
        sweep_limit: config.verification_sweep_limit,
        suspect_failures: config.verification_suspect_failures,
    }
}
async fn upgrade_control_channel(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    let lease = state
        .control_channels
        .register(&session_id)
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok(ws
        .on_upgrade(move |socket| {
            handle_control_channel(state, socket, authorized, lease, request_id)
        })
        .into_response())
}

async fn revoke_remote_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::RemoteSessionRevocationResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .revoke_remote_session(&session_id, &request_id)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    state
        .control_channels
        .revoke(&session_id, "revoked_by_admin");
    Ok(Json(response))
}

async fn authorize_session_headers(
    state: &AppState,
    headers: &HeaderMap,
    session_id: &str,
    request_id: &str,
    allow_terminal: bool,
) -> Result<AuthorizedSession, ApiError> {
    let credential = required_bearer_token(headers, request_id)?;
    let resume_token = required_header(headers, "x-burd-session-token", request_id)?;
    let device_id = required_header(headers, "x-burd-device-id", request_id)?;
    state
        .db
        .authorize_remote_session(
            session_id,
            &device_id,
            &credential,
            &resume_token,
            allow_terminal,
        )
        .await
        .map_err(|error| session_api_error(error, request_id.to_string()))
}

async fn handle_control_channel(
    state: Arc<AppState>,
    mut socket: WebSocket,
    authorized: AuthorizedSession,
    mut lease: ControlChannelLease,
    request_id: String,
) {
    let session_id = authorized.session_id.clone();
    if let Err(error) = state
        .db
        .mark_remote_session_connected(&session_id, &lease.connection_id)
        .await
    {
        let _ = send_control_error(&mut socket, &request_id, &session_id, error.to_string()).await;
        state
            .control_channels
            .release(&session_id, &lease.connection_id);
        return;
    }

    let ready = ServerControlMessage {
        request_id: request_id.clone(),
        session_id: session_id.clone(),
        sequence_ack: authorized.sequence_last,
        server_time: chrono::Utc::now().to_rfc3339(),
        message_type: "session_ready".to_string(),
        payload: serde_json::json!({
            "heartbeat_interval_seconds": authorized.heartbeat_interval_seconds,
            "missed_heartbeat_limit": authorized.missed_heartbeat_limit,
        }),
    };
    if send_server_message(&mut socket, &ready).await.is_err() {
        finish_control_channel(&state, &session_id, &lease.connection_id, "send_failed").await;
        return;
    }

    let timeout_seconds = u64::from(authorized.heartbeat_interval_seconds)
        * u64::from(authorized.missed_heartbeat_limit);
    let heartbeat_timeout = std::time::Duration::from_secs(timeout_seconds.max(1));
    let mut heartbeat_deadline = tokio::time::Instant::now() + heartbeat_timeout;
    let disconnect_reason = loop {
        tokio::select! {
            _ = tokio::time::sleep_until(heartbeat_deadline) => {
                break "missed_heartbeat_limit".to_string();
            }
            changed = lease.revocation.changed() => {
                let reason = if changed.is_ok() {
                    lease.revocation.borrow().clone().unwrap_or_else(|| "revoked".to_string())
                } else {
                    "revoked".to_string()
                };
                let message = ServerControlMessage {
                    request_id: new_request_id(),
                    session_id: session_id.clone(),
                    sequence_ack: authorized.sequence_last,
                    server_time: chrono::Utc::now().to_rfc3339(),
                    message_type: "session_revoked".to_string(),
                    payload: serde_json::json!({ "reason": reason }),
                };
                let _ = send_server_message(&mut socket, &message).await;
                break "revoked".to_string();
            }
            incoming = socket.recv() => {
                match incoming {
                    None => break "client_closed".to_string(),
                    Some(Err(_)) => break "socket_error".to_string(),
                    Some(Ok(Message::Close(_))) => break "client_closed".to_string(),
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break "socket_error".to_string();
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Binary(_))) => {
                        let _ = send_control_error(
                            &mut socket,
                            &new_request_id(),
                            &session_id,
                            "binary control messages are not supported".to_string(),
                        ).await;
                        break "invalid_message".to_string();
                    }
                    Some(Ok(Message::Text(text))) => {
                        let message = match serde_json::from_str::<ClientControlMessage>(&text) {
                            Ok(message) => message,
                            Err(error) => {
                                let _ = send_control_error(
                                    &mut socket,
                                    &new_request_id(),
                                    &session_id,
                                    format!("invalid control message: {error}"),
                                ).await;
                                break "invalid_message".to_string();
                            }
                        };
                        let control_request_id = new_request_id();
                        match message.message_type.as_str() {
                            "heartbeat" => {
                                match state.db.record_remote_heartbeat(
                                    &control_request_id,
                                    &authorized,
                                    &message,
                                    state.config.remote_session_ttl_seconds,
                                ).await {
                                    Ok(receipt) => {
                                        let response = ServerControlMessage {
                                            request_id: control_request_id,
                                            session_id: session_id.clone(),
                                            sequence_ack: receipt.sequence_ack,
                                            server_time: receipt.server_time.clone(),
                                            message_type: "heartbeat_ack".to_string(),
                                            payload: serde_json::to_value(receipt).unwrap_or_default(),
                                        };
                                        if send_server_message(&mut socket, &response).await.is_err() {
                                            break "send_failed".to_string();
                                        }
                                        heartbeat_deadline =
                                            tokio::time::Instant::now() + heartbeat_timeout;
                                    }
                                    Err(error) => {
                                        let _ = send_control_error(
                                            &mut socket,
                                            &control_request_id,
                                            &session_id,
                                            error.to_string(),
                                        ).await;
                                        break "heartbeat_rejected".to_string();
                                    }
                                }
                            }
                            "telemetry_batch" => {
                                match state.db.ingest_gpu_telemetry(
                                    &control_request_id,
                                    &authorized,
                                    &message,
                                    telemetry_policy(&state.config),
                                ).await {
                                    Ok(receipt) => {
                                        let response = ServerControlMessage {
                                            request_id: control_request_id,
                                            session_id: session_id.clone(),
                                            sequence_ack: receipt.control_sequence_ack,
                                            server_time: receipt.server_received_at.clone(),
                                            message_type: "telemetry_ack".to_string(),
                                            payload: serde_json::to_value(receipt).unwrap_or_default(),
                                        };
                                        if send_server_message(&mut socket, &response).await.is_err() {
                                            break "send_failed".to_string();
                                        }
                                    }
                                    Err(error) => {
                                        let sequence_ack = state
                                            .db
                                            .get_remote_session(
                                                &session_id,
                                                &authorized,
                                                &control_request_id,
                                            )
                                            .await
                                            .map(|session| session.sequence_last)
                                            .unwrap_or(0);
                                        let response = ServerControlMessage {
                                            request_id: control_request_id,
                                            session_id: session_id.clone(),
                                            sequence_ack,
                                            server_time: chrono::Utc::now().to_rfc3339(),
                                            message_type: "telemetry_rejected".to_string(),
                                            payload: serde_json::json!({
                                                "message": error.to_string(),
                                            }),
                                        };
                                        if send_server_message(&mut socket, &response).await.is_err() {
                                            break "send_failed".to_string();
                                        }
                                    }
                                }
                            }
                            _ => {
                                let _ = send_control_error(
                                    &mut socket,
                                    &control_request_id,
                                    &session_id,
                                    "unsupported control message type".to_string(),
                                ).await;
                                break "invalid_message".to_string();
                            }
                        }
                    }
                }
            }
        }
    };
    finish_control_channel(
        &state,
        &session_id,
        &lease.connection_id,
        &disconnect_reason,
    )
    .await;
}

async fn finish_control_channel(
    state: &AppState,
    session_id: &str,
    connection_id: &str,
    reason: &str,
) {
    let _ = state
        .db
        .mark_remote_session_disconnected(session_id, connection_id, reason)
        .await;
    state.control_channels.release(session_id, connection_id);
}

async fn send_server_message(
    socket: &mut WebSocket,
    message: &ServerControlMessage,
) -> Result<(), String> {
    let text = serde_json::to_string(message).map_err(|error| error.to_string())?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| error.to_string())
}

async fn send_control_error(
    socket: &mut WebSocket,
    request_id: &str,
    session_id: &str,
    error: String,
) -> Result<(), String> {
    send_server_message(
        socket,
        &ServerControlMessage {
            request_id: request_id.to_string(),
            session_id: session_id.to_string(),
            sequence_ack: 0,
            server_time: chrono::Utc::now().to_rfc3339(),
            message_type: "error".to_string(),
            payload: serde_json::json!({ "message": error }),
        },
    )
    .await
}
async fn rate_limit_middleware(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let key = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
        })
        .unwrap_or("local")
        .to_string();
    match state.rate_limiter.check(&key) {
        Ok(()) => next.run(request).await,
        Err(retry_after_seconds) => {
            ApiError::rate_limited(new_request_id(), retry_after_seconds).into_response()
        }
    }
}

fn required_idempotency_key(headers: &HeaderMap, request_id: &str) -> Result<String, ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            ApiError::invalid_request(
                "Idempotency-Key header is required for mutating requests",
                request_id.to_string(),
            )
        })
}

fn required_bearer_token(headers: &HeaderMap, request_id: &str) -> Result<String, ApiError> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::Unauthorized,
                "Authorization: Bearer credential is required",
                request_id,
            )
        })
}

fn required_header(
    headers: &HeaderMap,
    name: &'static str,
    request_id: &str,
) -> Result<String, ApiError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::Unauthorized,
                format!("{name} header is required"),
                request_id,
            )
        })
}

fn control_channel_url(headers: &HeaderMap) -> String {
    let host = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1:8080");
    let forwarded = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http");
    let scheme = if forwarded.eq_ignore_ascii_case("https") {
        "wss"
    } else {
        "ws"
    };
    format!("{scheme}://{host}/v1/sessions/{{session_id}}/control")
}
fn authorize_admin(
    headers: &HeaderMap,
    config: &ControlPlaneConfig,
    request_id: &str,
) -> Result<(), ApiError> {
    let token = required_bearer_token(headers, request_id)?;
    let candidate = sha256_hex(token.as_bytes());
    if constant_time_eq(candidate.as_bytes(), config.admin_token_hash.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthorized,
            "admin credential is invalid",
            request_id,
        ))
    }
}

fn enrollment_api_error(error: EnrollmentError, request_id: String) -> ApiError {
    match error {
        EnrollmentError::Database(error) => ApiError::database(error, request_id),
        EnrollmentError::NotFound(message) => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            message,
            request_id,
        ),
        EnrollmentError::Invalid(message) => ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            message,
            request_id,
        ),
        EnrollmentError::Unauthorized => ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthorized,
            "device or enrollment credential is invalid",
            request_id,
        ),
        EnrollmentError::Expired => ApiError::new(
            StatusCode::GONE,
            ErrorCode::Expired,
            "enrollment, nonce, or credential has expired",
            request_id,
        ),
        EnrollmentError::Revoked => ApiError::new(
            StatusCode::FORBIDDEN,
            ErrorCode::Revoked,
            "device, key, or enrollment has been revoked",
            request_id,
        ),
        EnrollmentError::NonceReused => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::NonceReused,
            "nonce has already been used",
            request_id,
        ),
        EnrollmentError::Conflict(message) => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::Conflict,
            message,
            request_id,
        ),
        EnrollmentError::SignatureInvalid => ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::SignatureInvalid,
            "Ed25519 proof signature is invalid",
            request_id,
        ),
    }
}

fn session_api_error(error: SessionError, request_id: String) -> ApiError {
    match error {
        SessionError::Database(error) => ApiError::database(error, request_id),
        SessionError::NotFound(message) => ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            message,
            request_id,
        ),
        SessionError::Invalid(message) => ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            message,
            request_id,
        ),
        SessionError::Unauthorized => ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthorized,
            "device or session credential is invalid",
            request_id,
        ),
        SessionError::Expired => ApiError::new(
            StatusCode::GONE,
            ErrorCode::Expired,
            "remote session or proof challenge has expired",
            request_id,
        ),
        SessionError::Revoked => ApiError::new(
            StatusCode::FORBIDDEN,
            ErrorCode::Revoked,
            "remote session or device has been revoked",
            request_id,
        ),
        SessionError::SignatureInvalid => ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::SignatureInvalid,
            "telemetry Ed25519 signature is invalid",
            request_id,
        ),
        SessionError::Conflict(message) => ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::Conflict,
            message,
            request_id,
        ),
    }
}
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn new_request_id() -> String {
    format!("req_{}", Uuid::new_v4())
}

fn migrations_are_current(applied: &[String], expected: &[&str]) -> bool {
    applied.len() == expected.len()
        && applied
            .iter()
            .map(String::as_str)
            .eq(expected.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    fn test_config(database_url: &str) -> ControlPlaneConfig {
        ControlPlaneConfig {
            environment: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 0,
            database_url: database_url.to_string(),
            database_schema: None,
            object_storage_dir: "target/test-control-objects".to_string(),
            rate_limit_per_minute: 120,
            admin_token_hash: sha256_hex(b"test-admin"),
            enrollment_token_ttl_seconds: 600,
            enrollment_proof_ttl_seconds: 300,
            device_credential_ttl_seconds: 900,
            remote_session_ttl_seconds: 900,
            heartbeat_interval_seconds: 15,
            missed_heartbeat_limit: 3,
            telemetry_max_samples_per_batch: 64,
            telemetry_min_batch_interval_seconds: 5,
            telemetry_clock_skew_seconds: 300,
            telemetry_retention_days: 7,
            proof_challenge_ttl_seconds: 600,
            proof_challenge_clock_skew_seconds: 300,
            verification_period_seconds: 3600,
            verification_retry_budget: 2,
            verification_sweep_limit: 25,
            verification_suspect_failures: 3,
        }
    }

    #[tokio::test]
    async fn health_endpoint_does_not_require_database_connection() {
        let config = test_config("postgres://localhost/unavailable");
        let db = Database::new(config.database_url.clone(), None).unwrap();
        let response = router(Arc::new(AppState::new(config, db)))
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["status"], "ok");
        assert_eq!(value["service"], "burd-control-plane");
    }

    #[tokio::test]
    async fn provider_creation_requires_admin_before_database_access() {
        let config = test_config("postgres://localhost/unavailable");
        let db = Database::new(config.database_url.clone(), None).unwrap();
        let response = router(Arc::new(AppState::new(config, db)))
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/providers")
                    .header("content-type", "application/json")
                    .header("idempotency-key", "provider-1")
                    .body(Body::from(r#"{"display_name":"Provider"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn create_provider_request_hash_is_stable() {
        let payload = CreateProviderRequest {
            user_id: None,
            display_name: Some("Provider".to_string()),
        };
        let hash = hash_canonical(&payload).unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn readiness_requires_every_expected_migration() {
        assert!(migrations_are_current(
            &[
                "0001".to_string(),
                "0002".to_string(),
                "0003".to_string(),
                "0004".to_string(),
                "0005".to_string(),
                "0006".to_string(),
                "0007".to_string()
            ],
            &["0001", "0002", "0003", "0004", "0005", "0006", "0007"]
        ));
        assert!(!migrations_are_current(
            &["0001".to_string()],
            &["0001", "0002", "0003", "0004", "0005", "0006", "0007"]
        ));
        assert!(!migrations_are_current(
            &[
                "0001".to_string(),
                "0002".to_string(),
                "unexpected".to_string()
            ],
            &["0001", "0002", "0003", "0004", "0005", "0006", "0007"]
        ));
    }

    #[test]
    fn admin_authorization_uses_hashed_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test-admin".parse().unwrap());
        let config = test_config("postgres://localhost/test");
        assert!(authorize_admin(&headers, &config, "req_test").is_ok());
        headers.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(authorize_admin(&headers, &config, "req_test").is_err());
    }
}
