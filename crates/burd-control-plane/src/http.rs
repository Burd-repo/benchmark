use crate::billing::{CreatePixPaymentIntentCommand, CreatePixPaymentIntentOutcome};
use crate::config::ControlPlaneConfig;
use crate::customer::{
    CreateReservationCommand, CreateReservationOutcome, CustomerApiKeyAuth,
    GrantCustomerCreditsCommand, GrantCustomerCreditsOutcome,
};
use crate::db::{CreateProviderCommand, CreateProviderOutcome, Database, ProviderRecord};
use crate::enrollment::EnrollmentError;
use crate::error::{ApiError, ErrorCode};
use crate::job_control::{CreateJobCommand, CreateJobOutcome};
use crate::observability::{
    ObservabilitySettings, ObservabilityState, ObservedHttpRequest, normalize_http_path,
};
use crate::openapi;
use crate::proof_challenge::ProofChallengePolicy;
use crate::rate_limit::RateLimiter;
use crate::remote_session::{
    AuthorizedSession, ControlChannelLease, ControlChannelRegistry, RemoteSessionPolicy,
    SessionError,
};
use crate::security_hardening::SecurityPolicy;
use crate::telemetry::TelemetryPolicy;
use crate::verification_policy::VerificationPolicy;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use burd_protocol::{
    AcceptJobRequest, CancelJobRequest, CancelReservationRequest, ClientControlMessage,
    ConfirmPixPaymentIntentRequest, CreateCustomerApiKeyRequest, CreateCustomerUserRequest,
    CreateJobRequest, CreateOrganizationRequest, CreatePixPaymentIntentRequest,
    CreateProjectRequest, CreateProviderPayoutRequest, CreateReservationRequest,
    EnrollmentProofRequest, GrantCustomerCreditsRequest, IssueProofChallengeRequest,
    JobEventRequest, KeyRotationProofRequest, RevokeEvidenceRequest,
    RunMarketplaceListingSweepRequest, RunSchedulerRequest, RunTrustSweepRequest,
    RunVerificationSweepRequest, RunWorkloadEligibilityRequest, ServerControlMessage,
    SettleReservationBillingRequest, SignedBenchmarkResult, SignedDeviceGpuInventory,
    SignedProofCapabilityResponse, SignedSecurityPosture, StartEnrollmentRequest,
    StartKeyRotationRequest, StartRemoteSessionRequest, SubmitEvidenceRequest,
    SubmitJobResultRequest, SubmitNetworkProbeObservationRequest, UpsertBenchmarkProfileRequest,
    UpsertMarketplacePriceRequest, UpsertProjectQuotaRequest, UpsertProviderPayoutAccountRequest,
    UpsertWorkloadPolicyRequest, hash_canonical, sha256_hex,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug)]
pub struct AppState {
    pub config: ControlPlaneConfig,
    pub db: Database,
    pub rate_limiter: RateLimiter,
    pub control_channels: ControlChannelRegistry,
    pub observability: ObservabilityState,
}

impl AppState {
    pub fn new(config: ControlPlaneConfig, db: Database) -> Self {
        let rate_limiter = RateLimiter::per_minute(config.rate_limit_per_minute);
        let observability = ObservabilityState::new(
            "burd-control-plane",
            config.environment.clone(),
            config.observability_deployment_id.clone(),
            ObservabilitySettings {
                recent_events_limit: config.observability_recent_events_limit,
                availability_target_bps: config.slo_availability_target_bps,
                p95_latency_ms: config.slo_p95_latency_ms,
            },
        );
        Self {
            config,
            db,
            rate_limiter,
            control_channels: ControlChannelRegistry::default(),
            observability,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CustomerListQuery {
    #[serde(default)]
    limit: Option<u32>,
}
#[derive(Debug, Clone, Deserialize)]
struct MarketplaceListingQuery {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    workload_type: Option<String>,
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

#[derive(Debug, Clone, Deserialize)]
struct NetworkProbeListQuery {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct AntifraudEventListQuery {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct BenchmarkResultListQuery {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct SecurityPostureListQuery {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceGpuInventoryListQuery {
    #[serde(default)]
    limit: Option<u32>,
}
#[derive(Debug, Clone, Deserialize)]
struct JobListQuery {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderEnvelope {
    request_id: String,
    audit_event_id: Option<String>,
    provider: ProviderRecord,
}

const MAX_IDEMPOTENCY_KEY_LENGTH: usize = 128;
const MAX_BEARER_TOKEN_LENGTH: usize = 4096;
const MAX_SESSION_HEADER_LENGTH: usize = 256;
const MAX_RATE_LIMIT_KEY_LENGTH: usize = 128;
const DEFAULT_CONTROL_CHANNEL_HOST: &str = "127.0.0.1:8080";
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/openapi.json", get(openapi_json))
        .route("/metrics", get(metrics))
        .route("/v1/observability/snapshot", get(observability_snapshot))
        .route("/v1/security/policy", get(security_policy_status))
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
            "/v1/sessions/{session_id}/security-posture",
            post(submit_security_posture),
        )
        .route(
            "/v1/sessions/{session_id}/gpu-inventory",
            post(submit_device_gpu_inventory),
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
        .route(
            "/v1/network-probes/observations",
            post(submit_network_probe_observation),
        )
        .route(
            "/v1/providers/{provider_id}/network-probes",
            get(list_network_probe_observations),
        )
        .route(
            "/v1/providers/{provider_id}/network-state",
            get(list_provider_network_states),
        )
        .route(
            "/v1/benchmark-profiles",
            get(list_benchmark_profiles).post(upsert_benchmark_profile),
        )
        .route(
            "/v1/workload-policies",
            get(list_workload_policies).post(upsert_workload_policy),
        )
        .route(
            "/v1/sessions/{session_id}/benchmark-results",
            post(submit_benchmark_result),
        )
        .route(
            "/v1/providers/{provider_id}/benchmark-results",
            get(list_provider_benchmark_results),
        )
        .route(
            "/v1/providers/{provider_id}/security-postures",
            get(list_provider_security_postures),
        )
        .route(
            "/v1/providers/{provider_id}/gpu-inventory",
            get(list_provider_device_gpu_inventory),
        )
        .route(
            "/v1/providers/{provider_id}/workload-eligibility",
            get(list_provider_workload_eligibility),
        )
        .route(
            "/v1/workload-eligibility/sweep",
            post(run_workload_eligibility_sweep),
        )
        .route("/v1/marketplace/listings", get(list_marketplace_listings))
        .route(
            "/v1/marketplace/listings/sweep",
            post(run_marketplace_listing_sweep),
        )
        .route(
            "/v1/marketplace/listings/{listing_id}/price",
            post(upsert_marketplace_price),
        )
        .route(
            "/v1/billing/projects/{project_id}/pix/payment-intents",
            post(create_pix_payment_intent),
        )
        .route(
            "/v1/billing/pix/payment-intents/{payment_intent_id}/confirm",
            post(confirm_pix_payment_intent),
        )
        .route(
            "/v1/billing/projects/{project_id}/balance",
            get(project_billing_balance),
        )
        .route(
            "/v1/billing/projects/{project_id}/ledger",
            get(list_project_financial_ledger),
        )
        .route(
            "/v1/billing/reservations/{reservation_id}/settle",
            post(settle_reservation_billing),
        )
        .route(
            "/v1/billing/invoices/{invoice_id}",
            get(get_billing_invoice),
        )
        .route(
            "/v1/billing/providers/{provider_id}/balance",
            get(provider_billing_balance),
        )
        .route(
            "/v1/billing/providers/{provider_id}/ledger",
            get(list_provider_financial_ledger),
        )
        .route(
            "/v1/billing/providers/{provider_id}/payout-account",
            post(upsert_provider_payout_account),
        )
        .route(
            "/v1/billing/providers/{provider_id}/payouts",
            post(create_provider_payout),
        )
        .route("/v1/customer/users", post(create_customer_user))
        .route("/v1/customer/organizations", post(create_organization))
        .route(
            "/v1/customer/organizations/{organization_id}",
            get(get_organization),
        )
        .route(
            "/v1/customer/organizations/{organization_id}/projects",
            post(create_project),
        )
        .route(
            "/v1/customer/organizations/{organization_id}/audit-events",
            get(list_customer_audit_events),
        )
        .route(
            "/v1/customer/projects/{project_id}/quotas",
            post(upsert_project_quota),
        )
        .route(
            "/v1/customer/projects/{project_id}/api-keys",
            post(create_customer_api_key),
        )
        .route(
            "/v1/customer/projects/{project_id}/credits",
            post(grant_customer_credits),
        )
        .route(
            "/v1/customer/projects/{project_id}/reservations",
            get(list_project_reservations).post(create_marketplace_reservation),
        )
        .route(
            "/v1/customer/projects/{project_id}/usage",
            get(customer_project_usage),
        )
        .route(
            "/v1/customer/reservations/{reservation_id}/cancel",
            post(cancel_marketplace_reservation),
        )
        .route("/v1/jobs", post(create_job))
        .route("/v1/jobs/{job_id}", get(get_job))
        .route("/v1/jobs/{job_id}/cancel", post(cancel_job))
        .route("/v1/jobs/{job_id}/usage-ledger", get(list_job_usage_ledger))
        .route(
            "/v1/jobs/{job_id}/usage-ledger/finalize",
            post(finalize_job_usage),
        )
        .route("/v1/jobs/{job_id}/leases", get(list_job_leases))
        .route("/v1/scheduler/run", post(run_scheduler))
        .route("/v1/providers/{provider_id}/jobs", get(list_provider_jobs))
        .route(
            "/v1/providers/{provider_id}/marketplace-listings",
            get(list_provider_marketplace_listings),
        )
        .route(
            "/v1/providers/{provider_id}/usage-ledger",
            get(list_provider_usage_ledger),
        )
        .route(
            "/v1/providers/{provider_id}/leases",
            get(list_provider_leases),
        )
        .route("/v1/sessions/{session_id}/jobs/next", get(next_job))
        .route(
            "/v1/sessions/{session_id}/jobs/{job_id}/accept",
            post(accept_job),
        )
        .route(
            "/v1/sessions/{session_id}/jobs/{job_id}/events",
            post(record_job_event),
        )
        .route(
            "/v1/sessions/{session_id}/jobs/{job_id}/result",
            post(submit_job_result),
        )
        .route("/v1/trust/sweep", post(run_trust_sweep))
        .route(
            "/v1/providers/{provider_id}/trust-states",
            get(list_provider_trust_states),
        )
        .route(
            "/v1/providers/{provider_id}/antifraud-events",
            get(list_antifraud_events),
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
        .layer(middleware::from_fn_with_state(
            state.clone(),
            observability_middleware,
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

async fn metrics(State(state): State<Arc<AppState>>) -> Response {
    (
        [
            ("content-type", "text/plain; version=0.0.4; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        state.observability.prometheus(),
    )
        .into_response()
}

async fn observability_snapshot(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<crate::observability::ObservabilitySnapshot>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    Ok(Json(state.observability.snapshot()))
}

async fn security_policy_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::SecurityPolicyStatusResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    Ok(Json(
        security_policy(&state.config).status_response(request_id),
    ))
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

async fn submit_security_posture(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<SignedSecurityPosture>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    let response = state
        .db
        .submit_security_posture(
            &request_id,
            &authorized,
            &payload,
            security_policy(&state.config),
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

async fn submit_device_gpu_inventory(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<SignedDeviceGpuInventory>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    let response = state
        .db
        .submit_device_gpu_inventory(&request_id, &authorized, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    let status = if response.duplicate {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(response)).into_response())
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

async fn submit_network_probe_observation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<SubmitNetworkProbeObservationRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .submit_network_probe_observation(&request_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    let status = if response.duplicate {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(response)).into_response())
}

async fn list_network_probe_observations(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<NetworkProbeListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListNetworkProbeObservationsResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_network_probe_observations(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_provider_network_states(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListProviderNetworkStatesResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_network_states(&request_id, &provider_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
async fn upsert_benchmark_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpsertBenchmarkProfileRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .upsert_benchmark_profile(&request_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn list_benchmark_profiles(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListBenchmarkProfilesResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_benchmark_profiles(&request_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn submit_benchmark_result(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<SignedBenchmarkResult>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    let response = state
        .db
        .submit_benchmark_result(&request_id, &authorized, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    let status = if response.duplicate {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(response)).into_response())
}

async fn list_provider_benchmark_results(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<BenchmarkResultListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListProviderBenchmarkResultsResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_benchmark_results(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_provider_security_postures(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<SecurityPostureListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListProviderSecurityPosturesResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_security_postures(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_provider_device_gpu_inventory(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<DeviceGpuInventoryListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListProviderDeviceGpuInventoryResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .list_provider_device_gpu_inventory(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map_err(|error| session_api_error(error, request_id))?;
    Ok(Json(response))
}
async fn upsert_workload_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpsertWorkloadPolicyRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .upsert_workload_policy(&request_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn list_workload_policies(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListWorkloadPoliciesResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_workload_policies(&request_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn run_workload_eligibility_sweep(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<RunWorkloadEligibilityRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .run_workload_eligibility_sweep(&request_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

async fn list_provider_workload_eligibility(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListProviderWorkloadEligibilityResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_workload_eligibility(&request_id, &provider_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
async fn run_marketplace_listing_sweep(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<RunMarketplaceListingSweepRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .run_marketplace_listing_sweep(&request_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

async fn list_marketplace_listings(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MarketplaceListingQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListMarketplaceListingsResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_marketplace_listings(
            &request_id,
            query.status.as_deref(),
            query.workload_type.as_deref(),
            query.limit.unwrap_or(50),
        )
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_provider_marketplace_listings(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<MarketplaceListingQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListMarketplaceListingsResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_marketplace_listings(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
async fn upsert_marketplace_price(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<UpsertMarketplacePriceRequest>,
) -> Result<Json<burd_protocol::MarketplacePriceResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .upsert_marketplace_price(&request_id, &listing_id, &payload)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn create_pix_payment_intent(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreatePixPaymentIntentRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let request_hash = hash_canonical(&payload)
        .map_err(|error| ApiError::invalid_request(error, request_id.clone()))?;
    let outcome = state
        .db
        .create_pix_payment_intent_idempotently(CreatePixPaymentIntentCommand {
            request_id: request_id.clone(),
            scope: format!("POST /v1/billing/projects/{project_id}/pix/payment-intents"),
            idempotency_key,
            request_hash,
            auth,
            project_id,
            request: payload,
        })
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    match outcome {
        CreatePixPaymentIntentOutcome::Response(record) => {
            let value = serde_json::from_str::<serde_json::Value>(&record.response_json).map_err(
                |error| ApiError::invalid_request(error.to_string(), request_id.clone()),
            )?;
            let status = StatusCode::from_u16(record.status_code).unwrap_or(StatusCode::OK);
            Ok((status, Json(value)).into_response())
        }
        CreatePixPaymentIntentOutcome::Conflict => Err(ApiError::idempotency_conflict(request_id)),
    }
}

async fn confirm_pix_payment_intent(
    State(state): State<Arc<AppState>>,
    Path(payment_intent_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<ConfirmPixPaymentIntentRequest>,
) -> Result<Json<burd_protocol::PixPaymentIntentResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .confirm_pix_payment_intent(&request_id, &payment_intent_id, &payload)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn project_billing_balance(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::BillingBalanceResponse>, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    state
        .db
        .project_billing_balance(&request_id, &auth, &project_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_project_financial_ledger(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Query(query): Query<CustomerListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::FinancialLedgerResponse>, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    state
        .db
        .list_project_financial_ledger(&request_id, &auth, &project_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn settle_reservation_billing(
    State(state): State<Arc<AppState>>,
    Path(reservation_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<SettleReservationBillingRequest>,
) -> Result<Json<burd_protocol::BillingInvoiceResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .settle_reservation_billing(&request_id, &reservation_id, &payload)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn get_billing_invoice(
    State(state): State<Arc<AppState>>,
    Path(invoice_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::BillingInvoiceResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .get_billing_invoice(&request_id, &invoice_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn provider_billing_balance(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::BillingBalanceResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .provider_billing_balance(&request_id, &provider_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_provider_financial_ledger(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<JobListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::FinancialLedgerResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_financial_ledger(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn upsert_provider_payout_account(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<UpsertProviderPayoutAccountRequest>,
) -> Result<Json<burd_protocol::ProviderPayoutAccountResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .upsert_provider_payout_account(&request_id, &provider_id, &payload)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn create_provider_payout(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateProviderPayoutRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .create_provider_payout(&request_id, &provider_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}
async fn create_customer_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateCustomerUserRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .create_customer_user(&request_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn create_organization(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateOrganizationRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .create_organization(&request_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn get_organization(
    State(state): State<Arc<AppState>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::OrganizationResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .get_organization(&request_id, &organization_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn create_project(
    State(state): State<Arc<AppState>>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateProjectRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .create_project(&request_id, &organization_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn upsert_project_quota(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<UpsertProjectQuotaRequest>,
) -> Result<Json<burd_protocol::ProjectQuotaResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .upsert_project_quota(&request_id, &project_id, &payload)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn create_customer_api_key(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateCustomerApiKeyRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .create_customer_api_key(&request_id, &project_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn grant_customer_credits(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<GrantCustomerCreditsRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let request_hash = hash_canonical(&payload)
        .map_err(|error| ApiError::invalid_request(error, request_id.clone()))?;
    let outcome = state
        .db
        .grant_customer_credits_idempotently(GrantCustomerCreditsCommand {
            request_id: request_id.clone(),
            scope: format!("POST /v1/customer/projects/{project_id}/credits"),
            idempotency_key,
            request_hash,
            project_id,
            request: payload,
        })
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    match outcome {
        GrantCustomerCreditsOutcome::Response(record) => {
            let value = serde_json::from_str::<serde_json::Value>(&record.response_json).map_err(
                |error| ApiError::invalid_request(error.to_string(), request_id.clone()),
            )?;
            let status = StatusCode::from_u16(record.status_code).unwrap_or(StatusCode::OK);
            Ok((status, Json(value)).into_response())
        }
        GrantCustomerCreditsOutcome::Conflict => Err(ApiError::idempotency_conflict(request_id)),
    }
}

async fn create_marketplace_reservation(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateReservationRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let request_hash = hash_canonical(&payload)
        .map_err(|error| ApiError::invalid_request(error, request_id.clone()))?;
    let outcome = state
        .db
        .create_marketplace_reservation_idempotently(CreateReservationCommand {
            request_id: request_id.clone(),
            scope: format!("POST /v1/customer/projects/{project_id}/reservations"),
            idempotency_key,
            request_hash,
            auth,
            project_id,
            request: payload,
        })
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    match outcome {
        CreateReservationOutcome::Response(record) => {
            let value = serde_json::from_str::<serde_json::Value>(&record.response_json).map_err(
                |error| ApiError::invalid_request(error.to_string(), request_id.clone()),
            )?;
            let status = StatusCode::from_u16(record.status_code).unwrap_or(StatusCode::OK);
            Ok((status, Json(value)).into_response())
        }
        CreateReservationOutcome::Conflict => Err(ApiError::idempotency_conflict(request_id)),
    }
}

async fn list_project_reservations(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Query(query): Query<CustomerListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListMarketplaceReservationsResponse>, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    state
        .db
        .list_project_reservations(&request_id, &auth, &project_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn customer_project_usage(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::CustomerUsageResponse>, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    state
        .db
        .customer_project_usage(&request_id, &auth, &project_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn cancel_marketplace_reservation(
    State(state): State<Arc<AppState>>,
    Path(reservation_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CancelReservationRequest>,
) -> Result<Json<burd_protocol::MarketplaceReservationResponse>, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, None, &request_id).await?;
    state
        .db
        .cancel_marketplace_reservation(&request_id, &auth, &reservation_id, &payload)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_customer_audit_events(
    State(state): State<Arc<AppState>>,
    Path(organization_id): Path<String>,
    Query(query): Query<CustomerListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListCustomerAuditEventsResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_customer_audit_events(&request_id, &organization_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
async fn create_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateJobRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let request_hash = hash_canonical(&payload)
        .map_err(|error| ApiError::invalid_request(error, request_id.clone()))?;

    let outcome = state
        .db
        .create_job_idempotently(CreateJobCommand {
            request_id: request_id.clone(),
            scope: "POST /v1/jobs".to_string(),
            idempotency_key,
            request_hash,
            request: payload,
        })
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;

    match outcome {
        CreateJobOutcome::Response(record) => {
            let value = serde_json::from_str::<serde_json::Value>(&record.response_json).map_err(
                |error| ApiError::invalid_request(error.to_string(), request_id.clone()),
            )?;
            let status = StatusCode::from_u16(record.status_code).unwrap_or(StatusCode::OK);
            Ok((status, Json(value)).into_response())
        }
        CreateJobOutcome::Conflict => Err(ApiError::idempotency_conflict(request_id)),
    }
}

async fn get_job(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::JobResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .get_job(&request_id, &job_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_provider_jobs(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<JobListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListJobsResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_jobs(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn run_scheduler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<RunSchedulerRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .run_scheduler(&request_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

async fn list_provider_leases(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<JobListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListJobLeasesResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_job_leases(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_job_leases(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListJobLeasesResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_job_leases(&request_id, &job_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
async fn finalize_job_usage(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .finalize_job_usage(&request_id, &job_id)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    let status = if response.duplicate {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(response)).into_response())
}

async fn list_job_usage_ledger(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListUsageLedgerResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_job_usage_ledger(&request_id, &job_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_provider_usage_ledger(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<JobListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListUsageLedgerResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_usage_ledger(&request_id, &provider_id, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
async fn next_job(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::NextJobResponse>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .next_job_for_session(&request_id, &authorized)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn accept_job(
    State(state): State<Arc<AppState>>,
    Path((session_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<AcceptJobRequest>,
) -> Result<Json<burd_protocol::JobResponse>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .accept_job(&request_id, &authorized, &job_id, &payload)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn record_job_event(
    State(state): State<Arc<AppState>>,
    Path((session_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<JobEventRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    let response = state
        .db
        .record_job_event(&request_id, &authorized, &job_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn submit_job_result(
    State(state): State<Arc<AppState>>,
    Path((session_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<SubmitJobResultRequest>,
) -> Result<Json<burd_protocol::SubmitJobResultResponse>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .submit_job_result(&request_id, &authorized, &job_id, &payload)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn cancel_job(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CancelJobRequest>,
) -> Result<Json<burd_protocol::JobResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .cancel_job(&request_id, &job_id, &payload)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}
async fn run_trust_sweep(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<RunTrustSweepRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .run_trust_sweep(&request_id, &payload)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

async fn list_provider_trust_states(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListProviderTrustStatesResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_trust_states(&request_id, &provider_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn list_antifraud_events(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<AntifraudEventListQuery>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListAntifraudEventsResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_antifraud_events(&request_id, &provider_id, query.limit.unwrap_or(50))
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

fn security_policy(config: &ControlPlaneConfig) -> SecurityPolicy {
    SecurityPolicy {
        min_agent_version: config.security_min_agent_version.clone(),
        require_signed_agent_release: config.security_require_signed_agent_release,
        require_hardware_backed_key: config.security_require_hardware_backed_key,
        require_remote_attestation: config.security_require_remote_attestation,
        require_sbom_hash: config.security_require_sbom_hash,
        accepted_release_channels: config.security_accepted_release_channels.clone(),
        accepted_attestation_modes: config.security_accepted_attestation_modes.clone(),
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

async fn authorize_customer_headers(
    state: &AppState,
    headers: &HeaderMap,
    project_id: Option<&str>,
    request_id: &str,
) -> Result<CustomerApiKeyAuth, ApiError> {
    let token = required_bearer_token(headers, request_id)?;
    state
        .db
        .authorize_customer_api_key(&token, project_id)
        .await
        .map_err(|error| session_api_error(error, request_id.to_string()))
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
async fn observability_middleware(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let started = Instant::now();
    let method = request.method().as_str().to_string();
    let path = normalize_http_path(request.uri().path());
    let correlation_id = correlation_id_from_headers(&headers);
    state.observability.begin_http_request();
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&correlation_id) {
        response
            .headers_mut()
            .insert("x-burd-correlation-id", value);
    }
    state
        .observability
        .finish_http_request(ObservedHttpRequest {
            correlation_id,
            method,
            path,
            status: response.status().as_u16(),
            duration_ms: started.elapsed().as_millis(),
        });
    response
}

async fn rate_limit_middleware(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let key = rate_limit_key_from_headers(&headers);
    match state.rate_limiter.check(&key) {
        Ok(()) => next.run(request).await,
        Err(retry_after_seconds) => {
            ApiError::rate_limited(new_request_id(), retry_after_seconds).into_response()
        }
    }
}

fn rate_limit_key_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| is_visible_ascii_without_whitespace(value, MAX_RATE_LIMIT_KEY_LENGTH))
        .filter(|value| !contains_secret_hint(value))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "local".to_string())
}

fn correlation_id_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-burd-correlation-id")
        .or_else(|| headers.get("x-request-id"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| is_visible_ascii_without_whitespace(value, 128))
        .filter(|value| !contains_secret_hint(value))
        .map(ToOwned::to_owned)
        .unwrap_or_else(new_request_id)
}

fn required_idempotency_key(headers: &HeaderMap, request_id: &str) -> Result<String, ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::invalid_request(
                "Idempotency-Key header is required for mutating requests",
                request_id.to_string(),
            )
        })?;
    if is_visible_ascii_without_whitespace(key, MAX_IDEMPOTENCY_KEY_LENGTH) {
        Ok(key.to_string())
    } else {
        Err(ApiError::invalid_request(
            "Idempotency-Key header must be 1-128 visible ASCII characters without whitespace",
            request_id.to_string(),
        ))
    }
}

fn required_bearer_token(headers: &HeaderMap, request_id: &str) -> Result<String, ApiError> {
    let Some(token) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_bearer_token)
    else {
        return Err(missing_or_malformed_bearer(request_id));
    };
    if is_visible_ascii_without_whitespace(token, MAX_BEARER_TOKEN_LENGTH) {
        Ok(token.to_string())
    } else {
        Err(missing_or_malformed_bearer(request_id))
    }
}

fn parse_bearer_token(value: &str) -> Option<&str> {
    let token = value.strip_prefix("Bearer ")?;
    if token.trim() == token && !token.is_empty() {
        Some(token)
    } else {
        None
    }
}

fn missing_or_malformed_bearer(request_id: &str) -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        ErrorCode::Unauthorized,
        "Authorization: Bearer credential is required",
        request_id,
    )
}

fn required_header(
    headers: &HeaderMap,
    name: &'static str,
    request_id: &str,
) -> Result<String, ApiError> {
    let value = headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::Unauthorized,
                format!("{name} header is required"),
                request_id,
            )
        })?;
    if is_visible_ascii_without_whitespace(value, MAX_SESSION_HEADER_LENGTH) {
        Ok(value.to_string())
    } else {
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthorized,
            format!("{name} header is malformed"),
            request_id,
        ))
    }
}

fn control_channel_url(headers: &HeaderMap) -> String {
    let host = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| is_safe_control_channel_host(value))
        .unwrap_or(DEFAULT_CONTROL_CHANNEL_HOST);
    let forwarded = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or("http");
    let scheme = if forwarded.eq_ignore_ascii_case("https") {
        "wss"
    } else {
        "ws"
    };
    format!("{scheme}://{host}/v1/sessions/{{session_id}}/control")
}

fn is_visible_ascii_without_whitespace(value: &str, max_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_length
        && value
            .chars()
            .all(|character| character.is_ascii_graphic() && !character.is_ascii_whitespace())
}

fn is_safe_control_channel_host(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .chars()
            .any(|character| character.is_ascii_alphanumeric())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | ':' | '[' | ']')
        })
}

fn contains_secret_hint(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "authorization",
        "bearer",
        "token",
        "secret",
        "password",
        "credential",
        "api_key",
        "private_key",
        "pix_key",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
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
    use axum::http::{HeaderValue, Method, Request};
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
            observability_deployment_id: "test".to_string(),
            observability_recent_events_limit: 16,
            slo_availability_target_bps: 9990,
            slo_p95_latency_ms: 500,
            security_min_agent_version: None,
            security_require_signed_agent_release: false,
            security_require_hardware_backed_key: false,
            security_require_remote_attestation: false,
            security_require_sbom_hash: false,
            security_accepted_release_channels: vec!["dev".to_string(), "stable".to_string()],
            security_accepted_attestation_modes: vec![
                "none".to_string(),
                "tpm".to_string(),
                "os_keychain".to_string(),
                "hsm".to_string(),
                "sev_snp".to_string(),
                "sgx".to_string(),
            ],
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
    async fn metrics_endpoint_reports_observed_requests_and_correlation_id() {
        let config = test_config("postgres://localhost/unavailable");
        let db = Database::new(config.database_url.clone(), None).unwrap();
        let app = router(Arc::new(AppState::new(config, db)));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health")
                    .header("x-burd-correlation-id", "corr-test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-burd-correlation-id")
                .unwrap()
                .to_str()
                .unwrap(),
            "corr-test"
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(text.contains("burd_control_plane_http_requests_total"));
        assert!(text.contains("deployment_id=\"test\""));
    }

    #[tokio::test]
    async fn security_policy_requires_admin_and_reports_defaults() {
        let config = test_config("postgres://localhost/unavailable");
        let db = Database::new(config.database_url.clone(), None).unwrap();
        let app = router(Arc::new(AppState::new(config, db)));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/security/policy")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/security/policy")
                    .header("authorization", "Bearer test-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["policy_version"], "burd-security-policy-v1");
        assert_eq!(value["require_remote_attestation"], false);
    }

    #[tokio::test]
    async fn observability_snapshot_requires_admin_and_reports_slo_state() {
        let config = test_config("postgres://localhost/unavailable");
        let db = Database::new(config.database_url.clone(), None).unwrap();
        let app = router(Arc::new(AppState::new(config, db)));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/observability/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/observability/snapshot")
                    .header("authorization", "Bearer test-admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["service"], "burd-control-plane");
        assert_eq!(value["environment"], "test");
        assert_eq!(value["slo"]["availability_target_bps"], 9990);
        assert!(value["http"]["total_requests"].as_u64().unwrap() >= 1);
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
                "0007".to_string(),
                "0008".to_string(),
                "0009".to_string(),
                "0010".to_string(),
                "0011".to_string()
            ],
            &[
                "0001", "0002", "0003", "0004", "0005", "0006", "0007", "0008", "0009", "0010",
                "0011"
            ]
        ));
        assert!(!migrations_are_current(
            &["0001".to_string()],
            &[
                "0001", "0002", "0003", "0004", "0005", "0006", "0007", "0008", "0009", "0010",
                "0011"
            ]
        ));
        assert!(!migrations_are_current(
            &[
                "0001".to_string(),
                "0002".to_string(),
                "unexpected".to_string()
            ],
            &[
                "0001", "0002", "0003", "0004", "0005", "0006", "0007", "0008", "0009", "0010",
                "0011"
            ]
        ));
    }

    #[test]
    fn bearer_and_idempotency_headers_are_strict_and_redacted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer test-admin"),
        );
        assert_eq!(
            required_bearer_token(&headers, "req_test").unwrap(),
            "test-admin"
        );

        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer test admin"),
        );
        let error = required_bearer_token(&headers, "req_test").unwrap_err();
        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
        assert!(!error.message.contains("test admin"));

        headers.insert("idempotency-key", HeaderValue::from_static("request-001"));
        assert_eq!(
            required_idempotency_key(&headers, "req_test").unwrap(),
            "request-001"
        );

        let long_key = "a".repeat(MAX_IDEMPOTENCY_KEY_LENGTH + 1);
        headers.insert("idempotency-key", HeaderValue::from_str(&long_key).unwrap());
        assert!(required_idempotency_key(&headers, "req_test").is_err());

        headers.insert("idempotency-key", HeaderValue::from_static("request 001"));
        assert!(required_idempotency_key(&headers, "req_test").is_err());
    }

    #[test]
    fn session_headers_and_reflected_control_channel_url_are_bounded() {
        let mut headers = HeaderMap::new();
        headers.insert("x-burd-device-id", HeaderValue::from_static("device_123"));
        assert_eq!(
            required_header(&headers, "x-burd-device-id", "req_test").unwrap(),
            "device_123"
        );
        headers.insert(
            "x-burd-session-token",
            HeaderValue::from_str(&"s".repeat(MAX_SESSION_HEADER_LENGTH + 1)).unwrap(),
        );
        assert!(required_header(&headers, "x-burd-session-token", "req_test").is_err());

        headers.insert("host", HeaderValue::from_static("control.burd.local:8443"));
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert_eq!(
            control_channel_url(&headers),
            "wss://control.burd.local:8443/v1/sessions/{session_id}/control"
        );

        headers.insert("host", HeaderValue::from_static("evil.example/path"));
        assert_eq!(
            control_channel_url(&headers),
            "wss://127.0.0.1:8080/v1/sessions/{session_id}/control"
        );
    }

    #[test]
    fn observability_header_values_do_not_persist_secret_like_inputs() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", HeaderValue::from_static("corr-test"));
        assert_eq!(correlation_id_from_headers(&headers), "corr-test");

        headers.insert(
            "x-request-id",
            HeaderValue::from_static("Bearer-token-leak"),
        );
        let generated = correlation_id_from_headers(&headers);
        assert!(generated.starts_with("req_"));
        assert_ne!(generated, "Bearer-token-leak");

        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.10, 10.0.0.2"),
        );
        assert_eq!(rate_limit_key_from_headers(&headers), "203.0.113.10");

        headers.insert("x-forwarded-for", HeaderValue::from_static("secret-token"));
        assert_eq!(rate_limit_key_from_headers(&headers), "local");
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
