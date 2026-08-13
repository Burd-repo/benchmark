use crate::billing::{CreatePixPaymentIntentCommand, CreatePixPaymentIntentOutcome};
use crate::config::ControlPlaneConfig;
use crate::customer::{
    CreateReservationCommand, CreateReservationOutcome, CustomerApiKeyAuth,
    GrantCustomerCreditsCommand, GrantCustomerCreditsOutcome,
};
use crate::customer_artifact::{CreateCustomerArtifactCommand, CreateCustomerArtifactOutcome};
use crate::customer_compute::{CreateCustomerWorkloadCommand, CreateCustomerWorkloadOutcome};
use crate::db::{CreateProviderCommand, CreateProviderOutcome, Database, ProviderRecord};
use crate::enrollment::EnrollmentError;
use crate::error::{ApiError, ErrorCode};
use crate::job_artifact::JobArtifactDirection;
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
use crate::runtime_admission::RuntimeAdmissionPolicy;
use crate::runtime_verification::RuntimeVerificationPolicy;
use crate::security_hardening::SecurityPolicy;
use crate::telemetry::TelemetryPolicy;
use crate::verification_policy::{RecurringProofProfile, VerificationPolicy};
use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use burd_protocol::{
    AcceptJobRequest, CancelJobRequest, CancelReservationRequest, ClientControlMessage,
    ConfirmPixPaymentIntentRequest, CreateCustomerApiKeyRequest, CreateCustomerArtifactRequest,
    CreateCustomerUserRequest, CreateCustomerWorkloadRequest, CreateJobRequest,
    CreateOrganizationRequest, CreatePixPaymentIntentRequest, CreateProjectRequest,
    CreateProviderPayoutRequest, CreateReservationRequest, EnrollmentProofRequest,
    GrantCustomerCreditsRequest, IssueProofChallengeRequest,
    IssueRuntimeVerificationChallengeRequest, JOB_ARTIFACT_UPLOAD_VERSION,
    JobArtifactUploadResponse, JobEventRequest, KeyRotationProofRequest, RevokeEvidenceRequest,
    RunMarketplaceListingSweepRequest, RunSchedulerRequest, RunTrustSweepRequest,
    RunVerificationSweepRequest, RunWorkloadEligibilityRequest, ServerControlMessage,
    SettleReservationBillingRequest, Sha256Accumulator, SignedBenchmarkResult,
    SignedDeviceGpuInventory, SignedProofCapabilityResponse, SignedProviderRuntimeObservation,
    SignedRuntimeVerificationResponse, SignedSecurityPosture, StartEnrollmentRequest,
    StartKeyRotationRequest, StartRemoteSessionRequest, SubmitEvidenceRequest,
    SubmitJobResultRequest, SubmitNetworkProbeObservationRequest, UpsertBenchmarkProfileRequest,
    UpsertMarketplacePriceRequest, UpsertProjectQuotaRequest, UpsertProviderPayoutAccountRequest,
    UpsertWorkloadPolicyRequest, hash_canonical, sha256_hex,
};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path as FilePath, PathBuf};
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

#[derive(Debug, Clone, Deserialize)]
struct JobExecutionControlQuery {
    lease_id: String,
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
            "/v1/customer/projects/{project_id}/workloads",
            post(create_customer_workload),
        )
        .route(
            "/v1/customer/projects/{project_id}/artifacts",
            post(create_customer_artifact),
        )
        .route(
            "/v1/customer/projects/{project_id}/artifacts/{artifact_id}/content",
            put(upload_customer_artifact),
        )
        .route(
            "/v1/customer/projects/{project_id}/artifacts/{artifact_id}/finalize",
            post(finalize_customer_artifact),
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
            "/v1/jobs/{job_id}/artifacts/{artifact_id}/download",
            get(download_job_artifact),
        )
        .route(
            "/v1/jobs/{job_id}/results/{artifact_id}/upload",
            put(upload_job_artifact),
        )
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
            "/v1/sessions/{session_id}/jobs/{job_id}/control",
            get(job_execution_control),
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
        .route(
            "/v1/runtime-verifications/challenges",
            post(issue_runtime_verification_challenge),
        )
        .route(
            "/v1/runtime-verifications/challenges/{challenge_id}",
            get(get_runtime_verification_challenge),
        )
        .route(
            "/v1/providers/{provider_id}/runtime-verifications",
            get(list_provider_runtime_verifications),
        )
        .route(
            "/v1/providers/{provider_id}/runtime-admissions",
            get(list_provider_runtime_admissions),
        )
        .route(
            "/v1/sessions/{session_id}/runtime-observations",
            post(submit_provider_runtime_observation),
        )
        .route(
            "/v1/sessions/{session_id}/runtime-verifications/next",
            get(next_runtime_verification_challenge),
        )
        .route(
            "/v1/sessions/{session_id}/runtime-verifications/{challenge_id}/response",
            post(submit_runtime_verification_response),
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

fn sensitive_json_response<T: Serialize>(status: StatusCode, value: T) -> Response {
    let mut response = (status, Json(value)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
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
    Ok(sensitive_json_response(StatusCode::CREATED, response))
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
    Ok(sensitive_json_response(StatusCode::CREATED, response))
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
    Ok(sensitive_json_response(StatusCode::CREATED, response))
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
    Ok(sensitive_json_response(StatusCode::CREATED, response))
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
    Ok(sensitive_json_response(StatusCode::CREATED, response))
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

async fn create_customer_workload(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateCustomerWorkloadRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let request_hash = hash_canonical(&payload)
        .map_err(|error| ApiError::invalid_request(error, request_id.clone()))?;
    let outcome = state
        .db
        .create_customer_workload_idempotently(
            CreateCustomerWorkloadCommand {
                request_id: request_id.clone(),
                scope: format!("POST /v1/customer/projects/{project_id}/workloads"),
                idempotency_key,
                request_hash,
                auth,
                project_id,
                request: payload,
            },
            &runtime_admission_policy(&state.config),
        )
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    match outcome {
        CreateCustomerWorkloadOutcome::Response(record) => {
            let value = serde_json::from_str::<serde_json::Value>(&record.response_json).map_err(
                |error| ApiError::invalid_request(error.to_string(), request_id.clone()),
            )?;
            let status = StatusCode::from_u16(record.status_code).unwrap_or(StatusCode::OK);
            Ok((status, Json(value)).into_response())
        }
        CreateCustomerWorkloadOutcome::Conflict => Err(ApiError::idempotency_conflict(request_id)),
    }
}

async fn create_customer_artifact(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateCustomerArtifactRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let request_hash = hash_canonical(&payload)
        .map_err(|error| ApiError::invalid_request(error, request_id.clone()))?;
    let outcome = state
        .db
        .create_customer_artifact_idempotently(CreateCustomerArtifactCommand {
            request_id: request_id.clone(),
            scope: format!("POST /v1/customer/projects/{project_id}/artifacts"),
            idempotency_key,
            request_hash,
            auth,
            project_id,
            request: payload,
        })
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    match outcome {
        CreateCustomerArtifactOutcome::Response(record) => {
            let value = serde_json::from_str::<serde_json::Value>(&record.response_json).map_err(
                |error| ApiError::invalid_request(error.to_string(), request_id.clone()),
            )?;
            let status = StatusCode::from_u16(record.status_code).unwrap_or(StatusCode::OK);
            Ok((status, Json(value)).into_response())
        }
        CreateCustomerArtifactOutcome::Conflict => Err(ApiError::idempotency_conflict(request_id)),
    }
}

async fn upload_customer_artifact(
    State(state): State<Arc<AppState>>,
    Path((project_id, artifact_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    let authorized = state
        .db
        .authorize_customer_artifact_upload(&auth, &project_id, &artifact_id)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    let expected = &authorized.artifact;
    let content_length = required_content_length(&headers, expected.size_bytes, &request_id)?;
    if content_length != expected.size_bytes {
        return Err(ApiError::invalid_request(
            "artifact Content-Length does not match its declaration",
            request_id,
        ));
    }
    let declared_digest = required_artifact_digest(&headers, &request_id)?;
    if declared_digest != expected.sha256 {
        return Err(ApiError::invalid_request(
            "artifact digest does not match its declaration",
            request_id,
        ));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream");
    if expected
        .content_type
        .as_deref()
        .is_some_and(|declared| declared != content_type)
    {
        return Err(ApiError::invalid_request(
            "artifact Content-Type does not match its declaration",
            request_id,
        ));
    }

    let destination =
        writable_object_path(&state.config.object_storage_dir, &authorized.object_key)
            .map_err(|_| artifact_storage_error(&request_id))?;
    let temporary =
        destination.with_file_name(format!(".burd-upload-{}.tmp", Uuid::new_v4().simple()));
    let (sender, receiver) = tokio::sync::mpsc::channel::<Bytes>(4);
    let writer_digest = declared_digest.clone();
    let writer = tokio::task::spawn_blocking(move || {
        write_upload_stream(
            &temporary,
            receiver,
            content_length,
            content_length,
            &writer_digest,
        )
    });
    let mut stream = body.into_data_stream();
    let mut received = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            ApiError::invalid_request("artifact upload body is invalid", request_id.clone())
        })?;
        received = received
            .checked_add(chunk.len() as u64)
            .filter(|size| *size <= content_length)
            .ok_or_else(|| {
                ApiError::invalid_request(
                    "artifact upload exceeds its declared size",
                    request_id.clone(),
                )
            })?;
        sender
            .send(chunk)
            .await
            .map_err(|_| artifact_storage_error(&request_id))?;
    }
    drop(sender);
    if received != content_length {
        return Err(ApiError::invalid_request(
            "artifact upload size does not match Content-Length",
            request_id,
        ));
    }
    let written = writer
        .await
        .map_err(|_| artifact_storage_error(&request_id))?
        .map_err(|_| artifact_storage_error(&request_id))?;
    finalize_uploaded_object(
        &written.temporary,
        &destination,
        &written.sha256,
        written.size_bytes,
    )
    .map_err(|_| artifact_storage_error(&request_id))?;
    let artifact = state
        .db
        .record_customer_artifact_upload(
            &auth,
            &project_id,
            &artifact_id,
            &written.sha256,
            written.size_bytes,
        )
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok(sensitive_json_response(
        StatusCode::OK,
        burd_protocol::CustomerArtifactResponse {
            request_id,
            artifact,
            duplicate: false,
        },
    ))
}

async fn finalize_customer_artifact(
    State(state): State<Arc<AppState>>,
    Path((project_id, artifact_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::CustomerArtifactResponse>, ApiError> {
    let request_id = new_request_id();
    let auth = authorize_customer_headers(&state, &headers, Some(&project_id), &request_id).await?;
    let authorized = state
        .db
        .authorize_customer_artifact_finalize(&auth, &project_id, &artifact_id)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    let path = existing_object_path(&state.config.object_storage_dir, &authorized.object_key)
        .map_err(|_| artifact_storage_error(&request_id))?;
    let expected_size = authorized.artifact.size_bytes;
    let expected_sha256 = authorized.artifact.sha256.clone();
    let object_matches = tokio::task::spawn_blocking(move || {
        let metadata = fs::metadata(&path)
            .map_err(|error| format!("artifact metadata inspection failed: {error}"))?;
        let stored_digest = hash_file(&path)?;
        Ok::<bool, String>(
            metadata.is_file()
                && metadata.len() == expected_size
                && stored_digest == expected_sha256,
        )
    })
    .await
    .map_err(|_| artifact_storage_error(&request_id))?
    .map_err(|_| artifact_storage_error(&request_id))?;
    if !object_matches {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::Conflict,
            "customer artifact object does not match its declaration",
            request_id,
        ));
    }
    state
        .db
        .finalize_customer_artifact(&request_id, &auth, &project_id, &artifact_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
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
        .run_scheduler(
            &request_id,
            &payload,
            &runtime_admission_policy(&state.config),
        )
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

async fn download_job_artifact(
    State(state): State<Arc<AppState>>,
    Path((job_id, artifact_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let credential = required_bearer_token(&headers, &request_id)?;
    let authorized = state
        .db
        .authorize_job_artifact(
            &job_id,
            &artifact_id,
            &credential,
            JobArtifactDirection::Download,
        )
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    let expected_size = authorized.artifact.size_bytes.ok_or_else(|| {
        ApiError::invalid_request("input artifact size_bytes is required", request_id.clone())
    })?;
    let expected_digest = authorized.artifact.sha256.as_deref().ok_or_else(|| {
        ApiError::invalid_request("input artifact sha256 is required", request_id.clone())
    })?;
    let path = existing_object_path(
        &state.config.object_storage_dir,
        &authorized.artifact.object_key,
    )
    .map_err(|_| artifact_storage_error(&request_id))?;
    let metadata = fs::metadata(&path).map_err(|_| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "input artifact object was not found",
            request_id.clone(),
        )
    })?;
    if !metadata.is_file() || metadata.len() != expected_size {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::Conflict,
            "input artifact object does not match its manifest",
            request_id,
        ));
    }
    let file = File::open(path).map_err(|_| artifact_storage_error(&request_id))?;
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(4);
    std::thread::spawn(move || stream_file_chunks(file, sender));
    let body_stream = stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    let content_type = authorized
        .artifact
        .content_type
        .as_deref()
        .unwrap_or("application/octet-stream");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, expected_size.to_string())
        .header(header::CONTENT_TYPE, content_type)
        .header("x-burd-content-sha256", expected_digest)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::PRAGMA, "no-cache")
        .header("x-content-type-options", "nosniff")
        .body(Body::from_stream(body_stream))
        .map_err(|_| artifact_storage_error(&request_id))
}

async fn upload_job_artifact(
    State(state): State<Arc<AppState>>,
    Path((job_id, artifact_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let credential = required_bearer_token(&headers, &request_id)?;
    let authorized = state
        .db
        .authorize_job_artifact(
            &job_id,
            &artifact_id,
            &credential,
            JobArtifactDirection::Upload,
        )
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    let maximum_size = authorized.artifact.size_bytes.ok_or_else(|| {
        ApiError::invalid_request(
            "expected output size_bytes limit is required",
            request_id.clone(),
        )
    })?;
    let content_length = required_content_length(&headers, maximum_size, &request_id)?;
    let declared_digest = required_artifact_digest(&headers, &request_id)?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    if authorized
        .artifact
        .content_type
        .as_deref()
        .is_some_and(|expected| expected != content_type)
    {
        return Err(ApiError::invalid_request(
            "artifact Content-Type does not match its manifest",
            request_id,
        ));
    }

    let destination = writable_object_path(
        &state.config.object_storage_dir,
        &authorized.artifact.object_key,
    )
    .map_err(|_| artifact_storage_error(&request_id))?;
    let temporary =
        destination.with_file_name(format!(".burd-upload-{}.tmp", Uuid::new_v4().simple()));
    let (sender, receiver) = tokio::sync::mpsc::channel::<Bytes>(4);
    let writer_digest = declared_digest.clone();
    let writer = tokio::task::spawn_blocking(move || {
        write_upload_stream(
            &temporary,
            receiver,
            content_length,
            maximum_size,
            &writer_digest,
        )
    });
    let mut stream = body.into_data_stream();
    let mut received = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            ApiError::invalid_request("artifact upload body is invalid", request_id.clone())
        })?;
        received = received
            .checked_add(chunk.len() as u64)
            .filter(|size| *size <= content_length && *size <= maximum_size)
            .ok_or_else(|| {
                ApiError::invalid_request(
                    "artifact upload exceeds its declared size",
                    request_id.clone(),
                )
            })?;
        sender
            .send(chunk)
            .await
            .map_err(|_| artifact_storage_error(&request_id))?;
    }
    drop(sender);
    if received != content_length {
        return Err(ApiError::invalid_request(
            "artifact upload size does not match Content-Length",
            request_id,
        ));
    }
    let written = writer
        .await
        .map_err(|_| artifact_storage_error(&request_id))?
        .map_err(|_| artifact_storage_error(&request_id))?;
    finalize_uploaded_object(
        &written.temporary,
        &destination,
        &written.sha256,
        written.size_bytes,
    )
    .map_err(|_| artifact_storage_error(&request_id))?;
    let artifact = state
        .db
        .record_job_artifact_upload(
            &job_id,
            &artifact_id,
            &credential,
            &written.sha256,
            written.size_bytes,
            Some(&content_type),
        )
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok(sensitive_json_response(
        StatusCode::OK,
        JobArtifactUploadResponse {
            schema_version: JOB_ARTIFACT_UPLOAD_VERSION.to_string(),
            request_id,
            job_id,
            artifact,
        },
    ))
}

struct WrittenUpload {
    temporary: PathBuf,
    sha256: String,
    size_bytes: u64,
}

fn write_upload_stream(
    temporary: &FilePath,
    mut receiver: tokio::sync::mpsc::Receiver<Bytes>,
    content_length: u64,
    maximum_size: u64,
    declared_digest: &str,
) -> Result<WrittenUpload, String> {
    let mut file = create_object_upload_file(temporary)?;
    let mut digest = Sha256Accumulator::new();
    let mut written = 0_u64;
    let result = (|| {
        while let Some(chunk) = receiver.blocking_recv() {
            written = written
                .checked_add(chunk.len() as u64)
                .filter(|size| *size <= content_length && *size <= maximum_size)
                .ok_or_else(|| "artifact upload exceeds its declared size".to_string())?;
            digest.update(&chunk);
            file.write_all(&chunk)
                .map_err(|error| format!("artifact upload write failed: {error}"))?;
        }
        if written != content_length {
            return Err("artifact upload size mismatch".to_string());
        }
        let sha256 = format!("sha256:{}", digest.finish_hex());
        if sha256 != declared_digest {
            return Err("artifact upload digest mismatch".to_string());
        }
        file.sync_all()
            .map_err(|error| format!("artifact upload sync failed: {error}"))?;
        Ok(WrittenUpload {
            temporary: temporary.to_path_buf(),
            sha256,
            size_bytes: written,
        })
    })();
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(windows)]
fn create_object_upload_file(path: &FilePath) -> Result<File, String> {
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("artifact upload file creation failed: {error}"))
}

#[cfg(not(windows))]
fn create_object_upload_file(path: &FilePath) -> Result<File, String> {
    burd_protocol::create_private_file_new(path)
}

fn stream_file_chunks(
    mut file: File,
    sender: tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
) {
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if sender
                    .blocking_send(Ok(Bytes::copy_from_slice(&buffer[..read])))
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.blocking_send(Err(error));
                break;
            }
        }
    }
}

fn required_content_length(
    headers: &HeaderMap,
    maximum: u64,
    request_id: &str,
) -> Result<u64, ApiError> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|size| *size <= maximum)
        .ok_or_else(|| {
            ApiError::invalid_request(
                "valid Content-Length within the artifact limit is required",
                request_id.to_string(),
            )
        })
}

fn required_artifact_digest(headers: &HeaderMap, request_id: &str) -> Result<String, ApiError> {
    let value = headers
        .get("x-burd-content-sha256")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let valid = value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if valid {
        Ok(value.to_ascii_lowercase())
    } else {
        Err(ApiError::invalid_request(
            "X-Burd-Content-Sha256 must use sha256:<64 hex>",
            request_id.to_string(),
        ))
    }
}

fn existing_object_path(root: &str, object_key: &str) -> Result<PathBuf, String> {
    let root = FilePath::new(root)
        .canonicalize()
        .map_err(|error| format!("object storage root is unavailable: {error}"))?;
    let candidate = object_path(&root, object_key)?
        .canonicalize()
        .map_err(|error| format!("artifact object is unavailable: {error}"))?;
    if !candidate.starts_with(&root) {
        return Err("artifact object escapes object storage".to_string());
    }
    Ok(candidate)
}

fn writable_object_path(root: &str, object_key: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|error| format!("object storage root failed: {error}"))?;
    let root = FilePath::new(root)
        .canonicalize()
        .map_err(|error| format!("object storage root is unavailable: {error}"))?;
    let candidate = object_path(&root, object_key)?;
    let relative = FilePath::new(object_key);
    let parent_components = relative
        .parent()
        .ok_or_else(|| "artifact object has no parent".to_string())?
        .components();
    let mut current = root.clone();
    for component in parent_components {
        let Component::Normal(component) = component else {
            return Err("artifact object key is unsafe".to_string());
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err("artifact object directory is unsafe".to_string());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|error| format!("artifact object directory failed: {error}"))?;
            }
            Err(error) => {
                return Err(format!("artifact object directory failed: {error}"));
            }
        }
        let canonical = current
            .canonicalize()
            .map_err(|error| format!("artifact object directory failed: {error}"))?;
        if !canonical.starts_with(&root) {
            return Err("artifact object escapes object storage".to_string());
        }
    }
    if fs::symlink_metadata(&candidate).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("artifact object path is a symbolic link".to_string());
    }
    Ok(candidate)
}

fn object_path(root: &FilePath, object_key: &str) -> Result<PathBuf, String> {
    let relative = FilePath::new(object_key);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("artifact object key is unsafe".to_string());
    }
    Ok(root.join(relative))
}

fn finalize_uploaded_object(
    temporary: &FilePath,
    destination: &FilePath,
    sha256: &str,
    size_bytes: u64,
) -> Result<(), String> {
    if destination.exists() {
        let metadata = fs::symlink_metadata(destination)
            .map_err(|error| format!("existing artifact inspection failed: {error}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != size_bytes
        {
            let _ = fs::remove_file(temporary);
            return Err("existing artifact conflicts with upload".to_string());
        }
        let existing_sha256 = hash_file(destination)?;
        if existing_sha256 != sha256 {
            let _ = fs::remove_file(temporary);
            return Err("existing artifact conflicts with upload".to_string());
        }
        fs::remove_file(temporary)
            .map_err(|error| format!("duplicate upload cleanup failed: {error}"))?;
        return Ok(());
    }
    fs::rename(temporary, destination)
        .map_err(|error| format!("artifact upload finalize failed: {error}"))
}

fn hash_file(path: &FilePath) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("artifact hash open failed: {error}"))?;
    let mut digest = Sha256Accumulator::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("artifact hash read failed: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", digest.finish_hex()))
}

fn artifact_storage_error(request_id: &str) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        ErrorCode::Internal,
        "artifact storage operation failed",
        request_id.to_string(),
    )
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
        .next_job_for_session(
            &request_id,
            &authorized,
            &runtime_admission_policy(&state.config),
        )
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
        .accept_job(
            &request_id,
            &authorized,
            &job_id,
            &payload,
            &runtime_admission_policy(&state.config),
        )
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn job_execution_control(
    State(state): State<Arc<AppState>>,
    Path((session_id, job_id)): Path<(String, String)>,
    query: Result<Query<JobExecutionControlQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let Query(query) = query.map_err(|_| {
        ApiError::invalid_request("lease_id query parameter is required", request_id.clone())
    })?;
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .job_execution_control(&request_id, &authorized, &job_id, &query.lease_id)
        .await
        .map(|response| sensitive_json_response(StatusCode::OK, response))
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
            recurring_proof_profile(&state.config),
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

async fn issue_runtime_verification_challenge(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<IssueRuntimeVerificationChallengeRequest>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let response = state
        .db
        .issue_runtime_verification_challenge(
            &request_id,
            &payload,
            runtime_verification_policy(&state.config),
        )
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn get_runtime_verification_challenge(
    State(state): State<Arc<AppState>>,
    Path(challenge_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::RuntimeVerificationChallengeRecord>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    let record = state
        .db
        .get_runtime_verification_challenge(&challenge_id)
        .await
        .map_err(|error| session_api_error(error, request_id.clone()))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "runtime verification challenge not found",
                request_id,
            )
        })?;
    Ok(Json(record))
}

async fn list_provider_runtime_verifications(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListProviderRuntimeVerificationsResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_runtime_verifications(&request_id, &provider_id)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn submit_provider_runtime_observation(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<SignedProviderRuntimeObservation>,
) -> Result<Response, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    let response = state
        .db
        .submit_provider_runtime_observation(
            &request_id,
            &authorized,
            &session_id,
            &payload,
            &runtime_admission_policy(&state.config),
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

async fn list_provider_runtime_admissions(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::ListProviderRuntimeAdmissionsResponse>, ApiError> {
    let request_id = new_request_id();
    authorize_admin(&headers, &state.config, &request_id)?;
    state
        .db
        .list_provider_runtime_admissions(
            &request_id,
            &provider_id,
            &runtime_admission_policy(&state.config),
        )
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn next_runtime_verification_challenge(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<burd_protocol::NextRuntimeVerificationChallengeResponse>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .next_runtime_verification_challenge(&request_id, &authorized)
        .await
        .map(Json)
        .map_err(|error| session_api_error(error, request_id))
}

async fn submit_runtime_verification_response(
    State(state): State<Arc<AppState>>,
    Path((session_id, challenge_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<SignedRuntimeVerificationResponse>,
) -> Result<Json<burd_protocol::SubmitRuntimeVerificationResponse>, ApiError> {
    let request_id = new_request_id();
    let authorized =
        authorize_session_headers(&state, &headers, &session_id, &request_id, false).await?;
    state
        .db
        .submit_runtime_verification_response(
            &request_id,
            &authorized,
            &session_id,
            &challenge_id,
            &payload,
            runtime_verification_policy(&state.config),
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
fn runtime_verification_policy(config: &ControlPlaneConfig) -> RuntimeVerificationPolicy {
    RuntimeVerificationPolicy {
        challenge_ttl_seconds: config.proof_challenge_ttl_seconds,
        clock_skew_seconds: config.proof_challenge_clock_skew_seconds,
        verification_ttl_seconds: config.verification_period_seconds.min(604_800),
        approved_proof_image_ref: config.runtime_proof_image_ref.clone(),
    }
}
fn runtime_admission_policy(config: &ControlPlaneConfig) -> RuntimeAdmissionPolicy {
    RuntimeAdmissionPolicy {
        clock_skew_seconds: config.proof_challenge_clock_skew_seconds,
        observation_max_age_seconds: config.runtime_observation_max_age_seconds,
        approved_proof_image_ref: config.runtime_proof_image_ref.clone(),
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

fn recurring_proof_profile(config: &ControlPlaneConfig) -> Option<RecurringProofProfile> {
    config
        .verification_proof_profile
        .as_ref()
        .map(|profile| RecurringProofProfile {
            profile_version: profile.profile_version.clone(),
            model_artifact_hash: profile.model_artifact_hash.clone(),
            required_proofs: profile.required_proofs.clone(),
            min_tokens_per_second: profile.min_tokens_per_second,
            max_ttft_ms: profile.max_ttft_ms,
        })
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
                let revocation_request_id = new_request_id();
                let sequence_ack = state
                    .db
                    .get_remote_session(&session_id, &authorized, &revocation_request_id)
                    .await
                    .map(|session| session.sequence_last)
                    .unwrap_or(authorized.sequence_last);
                let message = ServerControlMessage {
                    request_id: revocation_request_id,
                    session_id: session_id.clone(),
                    sequence_ack,
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
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
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
            verification_proof_profile: None,
            runtime_proof_image_ref: Some(format!(
                "ghcr.io/burd/runtime-proof@sha256:{}",
                "a".repeat(64)
            )),
            runtime_observation_max_age_seconds: 180,
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

    fn test_app(database_url: &str) -> Router {
        let config = test_config(database_url);
        let db = Database::new(config.database_url.clone(), None).unwrap();
        router(Arc::new(AppState::new(config, db)))
    }

    async fn send_request(
        app: Router,
        method: Method,
        uri: &str,
        body: Option<&str>,
        headers: &[(&str, &str)],
    ) -> axum::response::Response {
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        app.oneshot(
            builder
                .body(Body::from(body.unwrap_or_default().to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn assert_error_envelope(value: &serde_json::Value, code: &str) {
        assert_eq!(value["error"]["code"], code);
        assert!(value["error"]["message"].as_str().unwrap().len() > 2);
        assert!(
            value["error"]["request_id"]
                .as_str()
                .unwrap()
                .starts_with("req_")
        );
        assert!(value["error"]["details"].is_object());
    }

    #[test]
    fn sensitive_responses_disable_intermediary_caching() {
        let response = sensitive_json_response(
            StatusCode::CREATED,
            serde_json::json!({"token": "one-time"}),
        );

        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        assert_eq!(
            response.headers().get(header::PRAGMA),
            Some(&HeaderValue::from_static("no-cache"))
        );
    }

    #[tokio::test]
    async fn job_execution_control_query_failure_uses_bn00_error_envelope() {
        let response = send_request(
            test_app("postgres://localhost/unavailable"),
            Method::GET,
            "/v1/sessions/session_1/jobs/job_1/control",
            None,
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = response_json(response).await;
        assert_error_envelope(&value, "invalid_request");
        assert_eq!(
            value["error"]["message"],
            "lease_id query parameter is required"
        );
    }

    #[tokio::test]
    async fn live_router_serves_bn01_bn11_contract_paths_and_keeps_bn12_runtime_absent() {
        let app = test_app("postgres://localhost/unavailable");
        let openapi = send_request(app.clone(), Method::GET, "/openapi.json", None, &[]).await;
        assert_eq!(openapi.status(), StatusCode::OK);
        let document = response_json(openapi).await;

        for (method, live_uri, openapi_path, body) in [
            (Method::GET, "/health", "/health", None),
            (Method::GET, "/ready", "/ready", None),
            (Method::GET, "/openapi.json", "/openapi.json", None),
            (Method::GET, "/metrics", "/metrics", None),
            (
                Method::POST,
                "/v1/providers",
                "/v1/providers",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/providers/provider_live",
                "/v1/providers/{provider_id}",
                None,
            ),
            (
                Method::POST,
                "/v1/providers/provider_live/enrollment-tokens",
                "/v1/providers/{provider_id}/enrollment-tokens",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/providers/provider_live/devices",
                "/v1/providers/{provider_id}/devices",
                None,
            ),
            (
                Method::POST,
                "/v1/enrollments",
                "/v1/enrollments",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/enrollments/enrollment_live/proof",
                "/v1/enrollments/{enrollment_id}/proof",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/devices/device_live/credentials",
                "/v1/devices/{device_id}/credentials",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/devices/device_live/key-rotations",
                "/v1/devices/{device_id}/key-rotations",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/devices/device_live/key-rotations/rotation_live/proof",
                "/v1/devices/{device_id}/key-rotations/{rotation_id}/proof",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/devices/device_live/revoke",
                "/v1/devices/{device_id}/revoke",
                Some(r#"{}"#),
            ),
            (Method::POST, "/v1/sessions", "/v1/sessions", Some(r#"{}"#)),
            (
                Method::GET,
                "/v1/sessions/session_live",
                "/v1/sessions/{session_id}",
                None,
            ),
            (
                Method::GET,
                "/v1/sessions/session_live/control",
                "/v1/sessions/{session_id}/control",
                None,
            ),
            (
                Method::POST,
                "/v1/sessions/session_live/heartbeats",
                "/v1/sessions/{session_id}/heartbeats",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/sessions/session_live/revoke",
                "/v1/sessions/{session_id}/revoke",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/sessions/session_live/telemetry-batches",
                "/v1/sessions/{session_id}/telemetry-batches",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/sessions/session_live/telemetry/latest",
                "/v1/sessions/{session_id}/telemetry/latest",
                None,
            ),
            (
                Method::POST,
                "/v1/sessions/session_live/evidence-records",
                "/v1/sessions/{session_id}/evidence-records",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/providers/provider_live/evidence-records",
                "/v1/providers/{provider_id}/evidence-records",
                None,
            ),
            (
                Method::GET,
                "/v1/evidence-records/evidence_live",
                "/v1/evidence-records/{evidence_id}",
                None,
            ),
            (
                Method::POST,
                "/v1/evidence-records/evidence_live/revoke",
                "/v1/evidence-records/{evidence_id}/revoke",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/challenges",
                "/v1/challenges",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/challenges/challenge_live",
                "/v1/challenges/{challenge_id}",
                None,
            ),
            (
                Method::GET,
                "/v1/sessions/session_live/challenges/next",
                "/v1/sessions/{session_id}/challenges/next",
                None,
            ),
            (
                Method::POST,
                "/v1/sessions/session_live/challenges/challenge_live/response",
                "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/verification/sweep",
                "/v1/verification/sweep",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/providers/provider_live/verification-states",
                "/v1/providers/{provider_id}/verification-states",
                None,
            ),
            (
                Method::POST,
                "/v1/network-probes/observations",
                "/v1/network-probes/observations",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/providers/provider_live/network-probes",
                "/v1/providers/{provider_id}/network-probes",
                None,
            ),
            (
                Method::GET,
                "/v1/providers/provider_live/network-state",
                "/v1/providers/{provider_id}/network-state",
                None,
            ),
            (
                Method::POST,
                "/v1/trust/sweep",
                "/v1/trust/sweep",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/providers/provider_live/trust-states",
                "/v1/providers/{provider_id}/trust-states",
                None,
            ),
            (
                Method::GET,
                "/v1/providers/provider_live/antifraud-events",
                "/v1/providers/{provider_id}/antifraud-events",
                None,
            ),
            (
                Method::GET,
                "/v1/benchmark-profiles",
                "/v1/benchmark-profiles",
                None,
            ),
            (
                Method::POST,
                "/v1/benchmark-profiles",
                "/v1/benchmark-profiles",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/sessions/session_live/benchmark-results",
                "/v1/sessions/{session_id}/benchmark-results",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/providers/provider_live/benchmark-results",
                "/v1/providers/{provider_id}/benchmark-results",
                None,
            ),
            (
                Method::GET,
                "/v1/workload-policies",
                "/v1/workload-policies",
                None,
            ),
            (
                Method::POST,
                "/v1/workload-policies",
                "/v1/workload-policies",
                Some(r#"{}"#),
            ),
            (
                Method::POST,
                "/v1/workload-eligibility/sweep",
                "/v1/workload-eligibility/sweep",
                Some(r#"{}"#),
            ),
            (
                Method::GET,
                "/v1/providers/provider_live/workload-eligibility",
                "/v1/providers/{provider_id}/workload-eligibility",
                None,
            ),
        ] {
            let method_key = method.as_str().to_ascii_lowercase();
            assert!(
                document["paths"][openapi_path][method_key.as_str()].is_object(),
                "OpenAPI is missing {method_key} {openapi_path}"
            );
            let method_label = method.as_str().to_string();
            let response = send_request(app.clone(), method, live_uri, body, &[]).await;
            assert_ne!(
                response.status(),
                StatusCode::NOT_FOUND,
                "router returned 404 for {method_label} {live_uri}"
            );
            assert_ne!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "router returned 405 for {method_label} {live_uri}"
            );
        }

        for path in [
            "/v1/runtime",
            "/v1/runtime/jobs",
            "/v1/provider-runtime/jobs",
            "/v1/sessions/session_live/runtime/execute",
        ] {
            assert!(
                document["paths"].get(path).is_none(),
                "BN-12 runtime execution endpoint must not be documented: {path}"
            );
            let response = send_request(app.clone(), Method::POST, path, Some(r#"{}"#), &[]).await;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "BN-12 runtime execution endpoint must not be routed: {path}"
            );
        }
    }

    #[tokio::test]
    async fn live_protected_routes_return_redacted_error_envelopes_before_database_access() {
        let app = test_app("postgres://burd:secret-password@localhost/unavailable");
        for (method, uri) in [
            (Method::GET, "/v1/security/policy"),
            (Method::GET, "/v1/observability/snapshot"),
            (Method::GET, "/v1/providers/provider_live/devices"),
            (Method::GET, "/v1/sessions/session_live"),
            (Method::GET, "/v1/sessions/session_live/telemetry/latest"),
            (Method::GET, "/v1/sessions/session_live/challenges/next"),
            (Method::GET, "/v1/providers/provider_live/evidence-records"),
            (Method::GET, "/v1/providers/provider_live/network-state"),
            (Method::GET, "/v1/providers/provider_live/trust-states"),
            (Method::GET, "/v1/benchmark-profiles"),
            (Method::GET, "/v1/workload-policies"),
            (
                Method::GET,
                "/v1/customer/projects/project_live/reservations",
            ),
            (Method::GET, "/v1/billing/projects/project_live/balance"),
        ] {
            let response = send_request(app.clone(), method.clone(), uri, None, &[]).await;
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "expected 401 for {} {}",
                method.as_str(),
                uri
            );
            let value = response_json(response).await;
            assert_error_envelope(&value, "unauthorized");
            let serialized = value.to_string();
            assert!(!serialized.contains("secret-password"));
            assert!(!serialized.contains("postgres://"));
            assert!(!serialized.contains("test-admin"));
        }
    }

    #[tokio::test]
    async fn live_mutating_admin_routes_require_bounded_idempotency_keys_after_auth() {
        let app = test_app("postgres://burd:secret-password@localhost/unavailable");
        let provider_body = r#"{"display_name":"Provider"}"#;

        let response = send_request(
            app.clone(),
            Method::POST,
            "/v1/providers",
            Some(provider_body),
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = response_json(response).await;
        assert_error_envelope(&value, "invalid_request");
        assert!(
            value["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Idempotency-Key")
        );

        let long_key = "a".repeat(MAX_IDEMPOTENCY_KEY_LENGTH + 1);
        let response = send_request(
            app.clone(),
            Method::POST,
            "/v1/providers",
            Some(provider_body),
            &[
                ("authorization", "Bearer test-admin"),
                ("idempotency-key", long_key.as_str()),
            ],
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = response_json(response).await;
        assert_error_envelope(&value, "invalid_request");

        let response = send_request(
            app,
            Method::POST,
            "/v1/providers",
            Some(provider_body),
            &[
                ("authorization", "Bearer test-admin"),
                ("idempotency-key", "provider-live-db-unavailable"),
            ],
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let value = response_json(response).await;
        assert_error_envelope(&value, "database_unavailable");
        assert_eq!(value["error"]["details"]["reason"], "database_unavailable");
        let serialized = value.to_string();
        assert!(!serialized.contains("secret-password"));
        assert!(!serialized.contains("postgres://"));
    }

    #[tokio::test]
    #[ignore]
    async fn live_provider_registry_http_contract_persists_idempotency_and_readiness() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_http_contract_{}", Uuid::new_v4().simple());
        let mut config = test_config(&url);
        config.database_schema = Some(schema.clone());
        config.object_storage_dir = format!("target/test-control-objects/{schema}");
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();
        let app = router(Arc::new(AppState::new(config, db.clone())));

        let ready = send_request(app.clone(), Method::GET, "/ready", None, &[]).await;
        assert_eq!(ready.status(), StatusCode::OK);
        let ready = response_json(ready).await;
        assert_eq!(ready["status"], "ready");
        assert_eq!(ready["database"], "ok");
        assert_eq!(
            ready["migrations_applied"].as_array().unwrap().len(),
            crate::migrations::MIGRATIONS.len()
        );

        let body = r#"{"display_name":"Live Provider"}"#;
        let first = send_request(
            app.clone(),
            Method::POST,
            "/v1/providers",
            Some(body),
            &[
                ("authorization", "Bearer test-admin"),
                ("idempotency-key", "provider-live-http-1"),
            ],
        )
        .await;
        assert_eq!(first.status(), StatusCode::CREATED);
        let first = response_json(first).await;
        assert!(first["request_id"].as_str().unwrap().starts_with("req_"));
        let provider_id = first["provider"]["provider_id"].as_str().unwrap();
        assert_eq!(first["provider"]["display_name"], "Live Provider");
        assert_eq!(first["provider"]["status"], "unregistered");

        let replay = send_request(
            app.clone(),
            Method::POST,
            "/v1/providers",
            Some(body),
            &[
                ("authorization", "Bearer test-admin"),
                ("idempotency-key", "provider-live-http-1"),
            ],
        )
        .await;
        assert_eq!(replay.status(), StatusCode::CREATED);
        let replay = response_json(replay).await;
        assert_eq!(replay["provider"]["provider_id"], provider_id);

        let conflict = send_request(
            app.clone(),
            Method::POST,
            "/v1/providers",
            Some(r#"{"display_name":"Different Provider"}"#),
            &[
                ("authorization", "Bearer test-admin"),
                ("idempotency-key", "provider-live-http-1"),
            ],
        )
        .await;
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let conflict = response_json(conflict).await;
        assert_error_envelope(&conflict, "idempotency_conflict");

        let loaded = send_request(
            app,
            Method::GET,
            &format!("/v1/providers/{provider_id}"),
            None,
            &[],
        )
        .await;
        assert_eq!(loaded.status(), StatusCode::OK);
        let loaded = response_json(loaded).await;
        assert_eq!(loaded["provider"]["provider_id"], provider_id);

        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn live_customer_artifact_http_flow_uploads_verifies_and_finalizes_bytes() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_http_artifact_{}", Uuid::new_v4().simple());
        let object_storage_dir = format!("target/test-control-objects/{schema}");
        let mut config = test_config(&url);
        config.database_schema = Some(schema.clone());
        config.object_storage_dir = object_storage_dir.clone();
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();
        fs::create_dir_all(&object_storage_dir).unwrap();
        let client = db.connect().await.unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let customer_token = "customer-artifact-token";
        client
            .execute(
                "INSERT INTO organizations (organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ('org_http_artifact', 'burd-customer-organization-v1', 'Org', 'active', $1, $1)",
                &[&now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO projects (project_id, organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ('project_http_artifact', 'org_http_artifact', 'burd-customer-project-v1', 'Project', 'active', $1, $1)",
                &[&now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO customer_api_keys (api_key_id, organization_id, project_id, schema_version, key_prefix, key_hash, status, scopes_json, created_at) VALUES ('api_key_http_artifact', 'org_http_artifact', 'project_http_artifact', 'burd-customer-api-key-v1', 'customer-', $1, 'active', '[\"artifacts:write\"]', $2)",
                &[&sha256_hex(customer_token.as_bytes()), &now],
            )
            .await
            .unwrap();
        let app = router(Arc::new(AppState::new(config, db.clone())));
        let payload = b"customer-artifact-bytes";
        let digest = format!("sha256:{}", sha256_hex(payload));
        let create_body = serde_json::to_string(&serde_json::json!({
            "client_artifact_id": "customer-input-1",
            "sha256": digest,
            "size_bytes": payload.len(),
            "content_type": "application/octet-stream",
            "retention_seconds": 3600
        }))
        .unwrap();
        let authorization = format!("Bearer {customer_token}");
        let created = send_request(
            app.clone(),
            Method::POST,
            "/v1/customer/projects/project_http_artifact/artifacts",
            Some(&create_body),
            &[
                ("authorization", authorization.as_str()),
                ("idempotency-key", "customer-artifact-http-key"),
            ],
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = response_json(created).await;
        let artifact_id = created["artifact"]["artifact_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(created["artifact"]["status"], "pending_upload");
        assert!(created.get("object_key").is_none());
        assert!(!created.to_string().contains("credential"));

        let upload = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!(
                        "/v1/customer/projects/project_http_artifact/artifacts/{artifact_id}/content"
                    ))
                    .header("authorization", authorization.as_str())
                    .header("content-type", "application/octet-stream")
                    .header("content-length", payload.len())
                    .header("x-burd-content-sha256", digest.as_str())
                    .body(Body::from(payload.as_slice()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::OK);
        let upload = response_json(upload).await;
        assert_eq!(upload["artifact"]["status"], "uploaded");

        let finalize_uri =
            format!("/v1/customer/projects/project_http_artifact/artifacts/{artifact_id}/finalize");
        let finalized = send_request(
            app.clone(),
            Method::POST,
            &finalize_uri,
            None,
            &[("authorization", authorization.as_str())],
        )
        .await;
        assert_eq!(finalized.status(), StatusCode::OK);
        let finalized = response_json(finalized).await;
        assert_eq!(finalized["artifact"]["status"], "ready");
        assert_eq!(finalized["duplicate"], false);

        let replay = send_request(
            app,
            Method::POST,
            &finalize_uri,
            None,
            &[("authorization", authorization.as_str())],
        )
        .await;
        assert_eq!(replay.status(), StatusCode::OK);
        let replay = response_json(replay).await;
        assert_eq!(replay["duplicate"], true);

        db.drop_schema_for_test().await.unwrap();
        let _ = fs::remove_dir_all(object_storage_dir);
    }

    struct LiveHttpFixture {
        app: Router,
        db: Database,
        object_storage_dir: String,
        provider_id: String,
        device_id: String,
        session_id: String,
        resume_token: String,
        credential_authorization: String,
        public_key_id: String,
        keys: burd_protocol::KeyMaterial,
        local_provider_id: String,
        machine_id: String,
        hardware_fingerprint: String,
    }

    impl LiveHttpFixture {
        fn session_headers(&self) -> Vec<(&'static str, &str)> {
            vec![
                ("authorization", self.credential_authorization.as_str()),
                ("x-burd-session-token", self.resume_token.as_str()),
                ("x-burd-device-id", self.device_id.as_str()),
            ]
        }

        async fn cleanup(self) {
            self.db.drop_schema_for_test().await.unwrap();
            let _ = std::fs::remove_dir_all(&self.object_storage_dir);
        }
    }

    async fn live_enrolled_session_fixture(
        schema_label: &str,
        display_name: &str,
        idempotency_key: &str,
    ) -> LiveHttpFixture {
        live_enrolled_session_fixture_with_initial_heartbeat(
            schema_label,
            display_name,
            idempotency_key,
            true,
        )
        .await
    }

    async fn live_pending_session_fixture(
        schema_label: &str,
        display_name: &str,
        idempotency_key: &str,
    ) -> LiveHttpFixture {
        live_enrolled_session_fixture_with_initial_heartbeat(
            schema_label,
            display_name,
            idempotency_key,
            false,
        )
        .await
    }

    async fn live_enrolled_session_fixture_with_initial_heartbeat(
        schema_label: &str,
        display_name: &str,
        idempotency_key: &str,
        record_initial_heartbeat: bool,
    ) -> LiveHttpFixture {
        live_enrolled_session_fixture_with_config(
            schema_label,
            display_name,
            idempotency_key,
            record_initial_heartbeat,
            |_| {},
        )
        .await
    }

    async fn live_enrolled_session_fixture_with_config<F>(
        schema_label: &str,
        display_name: &str,
        idempotency_key: &str,
        record_initial_heartbeat: bool,
        configure: F,
    ) -> LiveHttpFixture
    where
        F: FnOnce(&mut ControlPlaneConfig),
    {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_http_{schema_label}_{}", Uuid::new_v4().simple());
        let object_storage_dir = format!("target/test-control-objects/{schema}");
        let mut config = test_config(&url);
        configure(&mut config);
        config.database_schema = Some(schema.clone());
        config.object_storage_dir = object_storage_dir.clone();
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();
        let app = router(Arc::new(AppState::new(config, db.clone())));

        let provider_body = serde_json::to_string(&serde_json::json!({
            "display_name": display_name
        }))
        .unwrap();
        let provider = send_request(
            app.clone(),
            Method::POST,
            "/v1/providers",
            Some(&provider_body),
            &[
                ("authorization", "Bearer test-admin"),
                ("idempotency-key", idempotency_key),
            ],
        )
        .await;
        assert_eq!(provider.status(), StatusCode::CREATED);
        let provider = response_json(provider).await;
        let provider_id = provider["provider"]["provider_id"]
            .as_str()
            .unwrap()
            .to_string();

        let token = send_request(
            app.clone(),
            Method::POST,
            &format!("/v1/providers/{provider_id}/enrollment-tokens"),
            Some(r#"{}"#),
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(token.status(), StatusCode::CREATED);
        let token = response_json(token).await;
        let enrollment_token = token["enrollment_token"].as_str().unwrap().to_string();
        assert!(!enrollment_token.contains("test-admin"));

        let keys = burd_protocol::generate_keypair().unwrap();
        let public_key = keys.public_key_base64.clone();
        let machine_id = format!("machine-{schema_label}");
        let local_provider_id = format!("local-provider-{schema_label}");
        let hardware_fingerprint = format!("sha256:{schema_label}-fingerprint");
        let start_body = serde_json::to_string(&serde_json::json!({
            "enrollment_token": enrollment_token,
            "public_key": &public_key,
            "key_algorithm": burd_protocol::KEY_ALGORITHM,
            "local_provider_id": &local_provider_id,
            "machine_id": &machine_id,
            "registration_payload": {
                "provider_id": &local_provider_id,
                "machine_id": &machine_id,
                "hardware_fingerprint": &hardware_fingerprint,
                "public_key": &public_key,
                "secrets_included": false
            },
            "hardware_fingerprint": &hardware_fingerprint,
            "agent_version": "burd-agent-test/0.1.0",
            "benchmark_version": "burd-bench-test/0.1.0"
        }))
        .unwrap();
        let started = send_request(
            app.clone(),
            Method::POST,
            "/v1/enrollments",
            Some(&start_body),
            &[],
        )
        .await;
        assert_eq!(started.status(), StatusCode::ACCEPTED);
        let started = response_json(started).await;
        assert_eq!(started["provider_id"], provider_id);
        let enrollment_id = started["enrollment_id"].as_str().unwrap().to_string();
        let nonce = started["nonce"].as_str().unwrap().to_string();
        let enrollment_expires_at = started["expires_at"].as_str().unwrap().to_string();

        let proof_message = burd_protocol::enrollment_proof_message(
            &enrollment_id,
            &provider_id,
            &machine_id,
            &nonce,
            &public_key,
            &hardware_fingerprint,
            &enrollment_expires_at,
        )
        .unwrap();
        let signature =
            burd_protocol::sign_message(&keys.secret_key_base64, proof_message.as_bytes()).unwrap();
        let proof_body = serde_json::to_string(&serde_json::json!({
            "nonce": nonce,
            "signature": signature,
            "public_key": &public_key,
            "hardware_fingerprint": &hardware_fingerprint
        }))
        .unwrap();
        let enrolled = send_request(
            app.clone(),
            Method::POST,
            &format!("/v1/enrollments/{enrollment_id}/proof"),
            Some(&proof_body),
            &[],
        )
        .await;
        assert_eq!(enrolled.status(), StatusCode::CREATED);
        let enrolled = response_json(enrolled).await;
        assert_eq!(enrolled["provider_id"], provider_id);
        assert_eq!(enrolled["status"], "pending_verification");
        let device_id = enrolled["device_id"].as_str().unwrap().to_string();
        let public_key_id = enrolled["public_key_id"].as_str().unwrap().to_string();
        let credential = enrolled["credential"].as_str().unwrap().to_string();
        let credential_authorization = format!("Bearer {credential}");

        let listed_devices = send_request(
            app.clone(),
            Method::GET,
            &format!("/v1/providers/{provider_id}/devices"),
            None,
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(listed_devices.status(), StatusCode::OK);
        let listed_devices = response_json(listed_devices).await;
        assert_eq!(listed_devices["devices"].as_array().unwrap().len(), 1);
        assert_eq!(listed_devices["devices"][0]["device_id"], device_id);
        assert_eq!(listed_devices["devices"][0]["status"], "active");

        let session_body = serde_json::to_string(&serde_json::json!({
            "provider_id": &provider_id,
            "device_id": &device_id,
            "hardware_fingerprint": &hardware_fingerprint,
            "agent_version": "burd-agent-test/0.1.0",
            "capabilities": {"backend": "cuda", "proof": "live-http-contract"},
            "latest_report_hash": null,
            "latest_challenge_id": null
        }))
        .unwrap();
        let started_session = send_request(
            app.clone(),
            Method::POST,
            "/v1/sessions",
            Some(&session_body),
            &[("authorization", credential_authorization.as_str())],
        )
        .await;
        assert_eq!(started_session.status(), StatusCode::CREATED);
        let started_session = response_json(started_session).await;
        assert_eq!(started_session["status"], "pending_connection");
        assert_eq!(started_session["sequence_start"], 0);
        let session_id = started_session["session_id"].as_str().unwrap().to_string();
        let resume_token = started_session["resume_token"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            started_session["control_url"]
                .as_str()
                .unwrap()
                .contains(&session_id)
        );

        let fixture = LiveHttpFixture {
            app,
            db,
            object_storage_dir,
            provider_id,
            device_id,
            session_id,
            resume_token,
            credential_authorization,
            public_key_id,
            keys,
            local_provider_id,
            machine_id,
            hardware_fingerprint,
        };
        if record_initial_heartbeat {
            let heartbeat_body = heartbeat_body(&fixture, 1);
            let headers = fixture.session_headers();
            let heartbeat = send_request(
                fixture.app.clone(),
                Method::POST,
                &format!("/v1/sessions/{}/heartbeats", fixture.session_id),
                Some(&heartbeat_body),
                &headers,
            )
            .await;
            assert_eq!(heartbeat.status(), StatusCode::OK);
            let heartbeat = response_json(heartbeat).await;
            assert_eq!(heartbeat["session_id"], fixture.session_id);
            assert_eq!(heartbeat["sequence_ack"], 1);
            assert_eq!(heartbeat["status"], "online");
            assert_eq!(heartbeat["next_heartbeat_seconds"], 15);
        }

        fixture
    }

    fn heartbeat_body(fixture: &LiveHttpFixture, sequence: u64) -> String {
        serde_json::to_string(&serde_json::json!({
            "session_id": &fixture.session_id,
            "device_id": &fixture.device_id,
            "sequence": sequence,
            "sent_at": chrono::Utc::now().to_rfc3339(),
            "type": "heartbeat",
            "payload": {
                "hardware_fingerprint": &fixture.hardware_fingerprint,
                "local_status": {"agent": "running", "source": "live-http-contract"}
            }
        }))
        .unwrap()
    }

    fn signed_telemetry_batch_for_fixture(
        fixture: &LiveHttpFixture,
        control_sequence: u64,
        sample_sequence: u64,
        gpu_uuid: &str,
    ) -> burd_protocol::SignedTelemetryBatch {
        let now = chrono::Utc::now().to_rfc3339();
        let payload = burd_protocol::TelemetryBatchPayload {
            schema_version: burd_protocol::TELEMETRY_SCHEMA_VERSION.to_string(),
            provider_id: fixture.provider_id.clone(),
            device_id: fixture.device_id.clone(),
            session_id: fixture.session_id.clone(),
            control_sequence,
            sample_sequence_start: sample_sequence,
            sample_sequence_end: sample_sequence,
            hardware_fingerprint: fixture.hardware_fingerprint.clone(),
            collector: "live-http-telemetry-contract".to_string(),
            collected_at_start: now.clone(),
            collected_at_end: now.clone(),
            samples: vec![burd_protocol::GpuTelemetrySample {
                sample_sequence,
                observed_at: now,
                gpu_uuid: gpu_uuid.to_string(),
                gpu_name: "NVIDIA RTX Live Test".to_string(),
                pci_bus_id: "00000000:01:00.0".to_string(),
                pci_vendor_id: Some("10de".to_string()),
                pci_device_id: Some("2684".to_string()),
                compute_capability: Some("8.9".to_string()),
                driver_version: "576.80".to_string(),
                cuda_driver_version: Some("12.9".to_string()),
                cuda_runtime_version: Some("12.9".to_string()),
                vram_total_mib: 24_576,
                vram_used_mib: Some(2_048),
                vram_free_mib: Some(22_528),
                gpu_utilization_percent: Some(63.0),
                memory_utilization_percent: Some(42.0),
                temperature_celsius: Some(64.0),
                power_draw_watts: Some(220.0),
                power_limit_watts: Some(320.0),
                graphics_clock_mhz: Some(1_800),
                sm_clock_mhz: Some(1_800),
                memory_clock_mhz: Some(10_500),
                performance_state: Some("P2".to_string()),
                throttle_reasons: Vec::new(),
                ecc_corrected_errors: None,
                ecc_uncorrected_errors: None,
                processes: vec![burd_protocol::GpuProcessTelemetry {
                    pid: 4242,
                    process_name: "burd-runtime".to_string(),
                    used_gpu_memory_mib: Some(1_024),
                    process_kind: "compute".to_string(),
                }],
                container_id: Some("container-live-telemetry".to_string()),
                job_id: None,
            }],
        };
        let batch_hash = burd_protocol::telemetry_batch_hash(&payload).unwrap();
        let signature_message = burd_protocol::telemetry_batch_signature_message(
            &payload,
            &batch_hash,
            &fixture.public_key_id,
        )
        .unwrap();
        let signature = burd_protocol::sign_message(
            &fixture.keys.secret_key_base64,
            signature_message.as_bytes(),
        )
        .unwrap();
        burd_protocol::SignedTelemetryBatch {
            payload,
            batch_hash,
            public_key_id: fixture.public_key_id.clone(),
            signature,
            canonicalization_version: burd_protocol::TELEMETRY_CANONICALIZATION_VERSION.to_string(),
        }
    }

    fn telemetry_batch_body(
        fixture: &LiveHttpFixture,
        control_sequence: u64,
        signed: &burd_protocol::SignedTelemetryBatch,
    ) -> String {
        serde_json::to_string(&serde_json::json!({
            "session_id": &fixture.session_id,
            "device_id": &fixture.device_id,
            "sequence": control_sequence,
            "sent_at": chrono::Utc::now().to_rfc3339(),
            "type": "telemetry_batch",
            "payload": signed
        }))
        .unwrap()
    }

    async fn submit_live_telemetry_batch(
        fixture: &LiveHttpFixture,
        control_sequence: u64,
        sample_sequence: u64,
        gpu_uuid: &str,
    ) -> burd_protocol::TelemetryBatchReceipt {
        let signed = signed_telemetry_batch_for_fixture(
            fixture,
            control_sequence,
            sample_sequence,
            gpu_uuid,
        );
        let body = telemetry_batch_body(fixture, control_sequence, &signed);
        let headers = fixture.session_headers();
        let submitted = send_request(
            fixture.app.clone(),
            Method::POST,
            &format!("/v1/sessions/{}/telemetry-batches", fixture.session_id),
            Some(&body),
            &headers,
        )
        .await;
        assert_eq!(submitted.status(), StatusCode::OK);
        let receipt: burd_protocol::TelemetryBatchReceipt =
            serde_json::from_value(response_json(submitted).await).unwrap();
        assert_eq!(receipt.batch_hash, signed.batch_hash);
        assert_eq!(receipt.control_sequence_ack, control_sequence);
        assert_eq!(receipt.sample_sequence_end, sample_sequence);

        let headers = fixture.session_headers();
        let latest = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/sessions/{}/telemetry/latest", fixture.session_id),
            None,
            &headers,
        )
        .await;
        assert_eq!(latest.status(), StatusCode::OK);
        let latest = response_json(latest).await;
        assert_eq!(latest["batch_hash"], receipt.batch_hash);
        assert_eq!(latest["samples"][0]["gpu_uuid"], gpu_uuid);
        receipt
    }
    fn live_control_channel_request(
        fixture: &LiveHttpFixture,
        addr: std::net::SocketAddr,
    ) -> tokio_tungstenite::tungstenite::http::Request<()> {
        let mut request = format!("ws://{addr}/v1/sessions/{}/control", fixture.session_id)
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            fixture.credential_authorization.parse().unwrap(),
        );
        request.headers_mut().insert(
            "x-burd-session-token",
            fixture.resume_token.parse().unwrap(),
        );
        request
            .headers_mut()
            .insert("x-burd-device-id", fixture.device_id.parse().unwrap());
        request
    }

    fn live_server_control_message(message: TungsteniteMessage) -> ServerControlMessage {
        match message {
            TungsteniteMessage::Text(text) => {
                let text = text.to_string();
                serde_json::from_str(&text).unwrap()
            }
            other => panic!("expected text control message, got {other:?}"),
        }
    }

    async fn spawn_live_http_server(
        app: Router,
    ) -> (
        std::net::SocketAddr,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });
        (addr, shutdown_tx, handle)
    }
    fn signed_report_for_fixture(fixture: &LiveHttpFixture) -> burd_protocol::SignedReport {
        let signed_at = chrono::Utc::now().to_rfc3339();
        let freshness =
            burd_protocol::evidence_freshness(&signed_at, burd_protocol::SIGNED_REPORT_TTL_SECONDS)
                .unwrap();
        let report = burd_protocol::FullReport {
            identity: None,
            evidence: Some(freshness.clone()),
            hardware_fingerprint: Some(fixture.hardware_fingerprint.clone()),
            marketplace_policy: None,
            system: serde_json::json!({
                "os": "linux",
                "machine_id": fixture.machine_id,
                "source": "live-http-contract"
            }),
            fit: None,
            llm_benchmark: None,
            stability: None,
            network: None,
            network_score: None,
            disk: None,
            reliability: None,
            ai_performance: None,
            score: serde_json::json!({"burd_compute_score": 0}),
            timestamp: signed_at.clone(),
            agent_version: "burd-agent-test/0.1.0".to_string(),
            benchmark_version: "burd-bench-test/0.1.0".to_string(),
            benchmark_profile: "live-http-contract".to_string(),
            challenge: None,
            signature: burd_protocol::ReportSignature {
                algorithm: burd_protocol::KEY_ALGORITHM.to_string(),
                value: "signed-report-envelope".to_string(),
                status: "signed".to_string(),
            },
        };
        let report_hash = burd_protocol::hash_canonical(&report).unwrap();
        let signature =
            burd_protocol::sign_message(&fixture.keys.secret_key_base64, report_hash.as_bytes())
                .unwrap();
        burd_protocol::SignedReport {
            provider_id: fixture.local_provider_id.clone(),
            machine_id: fixture.machine_id.clone(),
            report,
            report_hash,
            signature,
            public_key: fixture.keys.public_key_base64.clone(),
            key_algorithm: burd_protocol::KEY_ALGORITHM.to_string(),
            signed_at,
            evidence: Some(freshness),
            signature_valid_locally: true,
            canonicalization_version: burd_protocol::EVIDENCE_CANONICALIZATION_VERSION.to_string(),
        }
    }

    fn signed_proof_response_for_challenge(
        fixture: &LiveHttpFixture,
        challenge: &burd_protocol::ProofCapabilityChallenge,
        telemetry_window_hash: Option<String>,
    ) -> burd_protocol::SignedProofCapabilityResponse {
        let now = chrono::Utc::now().to_rfc3339();
        let payload = burd_protocol::ProofCapabilityResponsePayload {
            schema_version: burd_protocol::PROOF_CHALLENGE_RESPONSE_SCHEMA_VERSION.to_string(),
            challenge_id: challenge.challenge_id.clone(),
            nonce: challenge.nonce.clone(),
            provider_id: fixture.provider_id.clone(),
            device_id: fixture.device_id.clone(),
            session_id: fixture.session_id.clone(),
            profile_version: challenge.profile_version.clone(),
            hardware_fingerprint: fixture.hardware_fingerprint.clone(),
            gpu_uuid: challenge
                .required_gpu_uuid
                .clone()
                .unwrap_or_else(|| "GPU-live-http".to_string()),
            backend: challenge.required_backend.clone(),
            model_artifact_hash: challenge.model_artifact_hash.clone(),
            prompt_seed: challenge.prompt_seed.clone(),
            driver_version: "576.80".to_string(),
            cuda_driver_version: Some("12.9".to_string()),
            cuda_runtime_version: Some("12.9".to_string()),
            metrics: burd_protocol::ProofCapabilityMetrics {
                tokens_per_second: Some(48.0),
                ttft_ms: Some(120),
                vram_allocated_mib: Some(1024),
                vram_resident_mib: Some(768),
                gemm_gflops: Some(125.0),
                cuda_runtime_detected: true,
                backend_proof: "cuda_runtime_detected".to_string(),
                contention_detected: false,
            },
            telemetry_window_hash,
            started_at: now.clone(),
            completed_at: now,
        };
        let response_hash = burd_protocol::proof_capability_response_hash(&payload).unwrap();
        let signature_message = burd_protocol::proof_capability_response_signature_message(
            &payload,
            &response_hash,
            &fixture.public_key_id,
        )
        .unwrap();
        let signature = burd_protocol::sign_message(
            &fixture.keys.secret_key_base64,
            signature_message.as_bytes(),
        )
        .unwrap();
        burd_protocol::SignedProofCapabilityResponse {
            payload,
            response_hash,
            public_key_id: fixture.public_key_id.clone(),
            signature,
            canonicalization_version: burd_protocol::PROOF_CHALLENGE_CANONICALIZATION_VERSION
                .to_string(),
        }
    }

    async fn issue_live_proof_challenge(
        fixture: &LiveHttpFixture,
        profile_version: &str,
        prompt_seed: &str,
    ) -> burd_protocol::ProofCapabilityChallenge {
        let issue_body = serde_json::to_string(&serde_json::json!({
            "provider_id": &fixture.provider_id,
            "device_id": &fixture.device_id,
            "session_id": &fixture.session_id,
            "profile_version": profile_version,
            "required_fingerprint": &fixture.hardware_fingerprint,
            "required_gpu_uuid": "GPU-live-http",
            "required_backend": "cuda",
            "model_artifact_hash": "sha256:live-proof-model-artifact",
            "prompt_seed": prompt_seed,
            "required_proofs": [
                "cuda_runtime",
                "vram_allocation_residency",
                "tensor_gemm_microbenchmark",
                "llm_short_inference",
                "contention_detection",
                "telemetry_window"
            ],
            "min_tokens_per_second": 1.0,
            "max_ttft_ms": 500,
            "expires_in_seconds": 300
        }))
        .unwrap();
        let issued = send_request(
            fixture.app.clone(),
            Method::POST,
            "/v1/challenges",
            Some(&issue_body),
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(issued.status(), StatusCode::CREATED);
        let issued = response_json(issued).await;
        let challenge: burd_protocol::ProofCapabilityChallenge =
            serde_json::from_value(issued["challenge"].clone()).unwrap();
        assert_eq!(challenge.provider_id, fixture.provider_id);
        assert_eq!(challenge.device_id, fixture.device_id);
        assert_eq!(challenge.session_id, fixture.session_id);
        challenge
    }
    #[tokio::test]
    #[ignore]
    async fn live_control_channel_websocket_flow_persists_telemetry_and_rejects_replay() {
        let fixture = live_pending_session_fixture(
            "control_ws",
            "Live Control Channel Provider",
            "provider-live-control-ws",
        )
        .await;
        let (addr, shutdown, server) = spawn_live_http_server(fixture.app.clone()).await;

        let (mut socket, response) =
            tokio_tungstenite::connect_async(live_control_channel_request(&fixture, addr))
                .await
                .unwrap();
        assert_eq!(response.status().as_u16(), 101);

        let ready = live_server_control_message(socket.next().await.unwrap().unwrap());
        assert_eq!(ready.session_id, fixture.session_id);
        assert_eq!(ready.message_type, "session_ready");
        assert_eq!(ready.sequence_ack, 0);
        assert_eq!(ready.payload["heartbeat_interval_seconds"], 15);
        assert_eq!(ready.payload["missed_heartbeat_limit"], 3);

        let headers = fixture.session_headers();
        let connected = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/sessions/{}", fixture.session_id),
            None,
            &headers,
        )
        .await;
        assert_eq!(connected.status(), StatusCode::OK);
        let connected = response_json(connected).await;
        assert_eq!(connected["status"], "online");
        assert_eq!(connected["sequence_last"], 0);
        assert!(connected["connected_at"].is_string());

        let duplicate =
            tokio_tungstenite::connect_async(live_control_channel_request(&fixture, addr)).await;
        assert!(
            duplicate.is_err(),
            "a second live control channel for the same session must not upgrade"
        );

        socket
            .send(TungsteniteMessage::Text(heartbeat_body(&fixture, 1).into()))
            .await
            .unwrap();
        let ack = live_server_control_message(socket.next().await.unwrap().unwrap());
        assert_eq!(ack.session_id, fixture.session_id);
        assert_eq!(ack.message_type, "heartbeat_ack");
        assert_eq!(ack.sequence_ack, 1);
        assert_eq!(ack.payload["status"], "online");
        assert_eq!(ack.payload["sequence_ack"], 1);

        let signed = signed_telemetry_batch_for_fixture(&fixture, 2, 1, "GPU-live-websocket");
        let telemetry_body = telemetry_batch_body(&fixture, 2, &signed);
        socket
            .send(TungsteniteMessage::Text(telemetry_body.clone().into()))
            .await
            .unwrap();
        let telemetry_ack = live_server_control_message(socket.next().await.unwrap().unwrap());
        assert_eq!(telemetry_ack.session_id, fixture.session_id);
        assert_eq!(telemetry_ack.message_type, "telemetry_ack");
        assert_eq!(telemetry_ack.sequence_ack, 2);
        let telemetry_receipt: burd_protocol::TelemetryBatchReceipt =
            serde_json::from_value(telemetry_ack.payload).unwrap();
        assert_eq!(telemetry_receipt.status, "accepted");
        assert_eq!(telemetry_receipt.control_sequence_ack, 2);
        assert_eq!(telemetry_receipt.sample_sequence_end, 1);
        assert_eq!(telemetry_receipt.sample_count, 1);
        assert_eq!(telemetry_receipt.batch_hash, signed.batch_hash);

        socket
            .send(TungsteniteMessage::Text(telemetry_body.into()))
            .await
            .unwrap();
        let telemetry_replay = live_server_control_message(socket.next().await.unwrap().unwrap());
        assert_eq!(telemetry_replay.session_id, fixture.session_id);
        assert_eq!(telemetry_replay.message_type, "telemetry_rejected");
        assert_eq!(telemetry_replay.sequence_ack, 2);
        assert!(
            telemetry_replay.payload["message"]
                .as_str()
                .unwrap()
                .contains("already observed")
        );

        let headers = fixture.session_headers();
        let latest = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/sessions/{}/telemetry/latest", fixture.session_id),
            None,
            &headers,
        )
        .await;
        assert_eq!(latest.status(), StatusCode::OK);
        let latest = response_json(latest).await;
        assert_eq!(latest["batch_hash"], signed.batch_hash);
        assert_eq!(latest["samples"][0]["gpu_uuid"], "GPU-live-websocket");

        let client = fixture.db.connect().await.unwrap();
        let persisted = client
            .query_one(
                "SELECT t.control_sequence, t.sample_sequence_start, t.sample_sequence_end, t.sample_count, (SELECT COUNT(*)::BIGINT FROM gpu_telemetry_samples s WHERE s.batch_id = t.batch_id) AS persisted_sample_count, (SELECT s.gpu_uuid FROM gpu_telemetry_samples s WHERE s.batch_id = t.batch_id ORDER BY s.sample_sequence LIMIT 1) AS gpu_uuid FROM telemetry_batches t WHERE t.session_id = $1 AND t.batch_hash = $2",
                &[&fixture.session_id, &signed.batch_hash],
            )
            .await
            .unwrap();
        assert_eq!(persisted.get::<_, i64>("control_sequence"), 2);
        assert_eq!(persisted.get::<_, i64>("sample_sequence_start"), 1);
        assert_eq!(persisted.get::<_, i64>("sample_sequence_end"), 1);
        assert_eq!(persisted.get::<_, i32>("sample_count"), 1);
        assert_eq!(persisted.get::<_, i64>("persisted_sample_count"), 1);
        assert_eq!(persisted.get::<_, String>("gpu_uuid"), "GPU-live-websocket");

        let revoked = send_request(
            fixture.app.clone(),
            Method::POST,
            &format!("/v1/sessions/{}/revoke", fixture.session_id),
            None,
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(revoked.status(), StatusCode::OK);
        let revoked = response_json(revoked).await;
        assert_eq!(revoked["status"], "revoked");

        let revocation = live_server_control_message(socket.next().await.unwrap().unwrap());
        assert_eq!(revocation.session_id, fixture.session_id);
        assert_eq!(revocation.message_type, "session_revoked");
        assert_eq!(revocation.sequence_ack, 2);
        assert_eq!(revocation.payload["reason"], "revoked_by_admin");

        drop(socket);
        let _ = shutdown.send(());
        server.await.unwrap();
        fixture.cleanup().await;
    }
    #[tokio::test]
    #[ignore]
    async fn live_enrollment_and_remote_session_http_flow_persists_authoritative_state() {
        let fixture = live_enrolled_session_fixture(
            "enrollment_session",
            "Live Enrollment Session Provider",
            "provider-live-enrollment-session",
        )
        .await;

        let replay_body = heartbeat_body(&fixture, 1);
        let headers = fixture.session_headers();
        let replay = send_request(
            fixture.app.clone(),
            Method::POST,
            &format!("/v1/sessions/{}/heartbeats", fixture.session_id),
            Some(&replay_body),
            &headers,
        )
        .await;
        assert_eq!(replay.status(), StatusCode::CONFLICT);
        let replay = response_json(replay).await;
        assert_error_envelope(&replay, "conflict");

        let headers = fixture.session_headers();
        let loaded_session = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/sessions/{}", fixture.session_id),
            None,
            &headers,
        )
        .await;
        assert_eq!(loaded_session.status(), StatusCode::OK);
        let loaded_session = response_json(loaded_session).await;
        assert_eq!(loaded_session["provider_id"], fixture.provider_id);
        assert_eq!(loaded_session["device_id"], fixture.device_id);
        assert_eq!(loaded_session["status"], "online");
        assert_eq!(loaded_session["sequence_last"], 1);
        assert!(loaded_session["last_seen_at"].is_string());

        let client = fixture.db.connect().await.unwrap();
        let persisted = client
            .query_one(
                "SELECT p.status AS provider_status, d.status AS device_status, s.status AS session_status, s.sequence_last, COUNT(h.heartbeat_id)::BIGINT AS heartbeat_count FROM providers p JOIN devices d ON d.provider_id = p.provider_id JOIN provider_sessions s ON s.provider_id = p.provider_id AND s.device_id = d.device_id LEFT JOIN session_heartbeats h ON h.session_id = s.session_id WHERE p.provider_id = $1 AND d.device_id = $2 AND s.session_id = $3 GROUP BY p.status, d.status, s.status, s.sequence_last",
                &[&fixture.provider_id, &fixture.device_id, &fixture.session_id],
            )
            .await
            .unwrap();
        assert_eq!(
            persisted.get::<_, String>("provider_status"),
            "pending_verification"
        );
        assert_eq!(persisted.get::<_, String>("device_status"), "active");
        assert_eq!(persisted.get::<_, String>("session_status"), "online");
        assert_eq!(persisted.get::<_, i64>("sequence_last"), 1);
        assert_eq!(persisted.get::<_, i64>("heartbeat_count"), 1);

        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn live_evidence_registry_http_flow_persists_valid_evidence_and_deduplicates() {
        let fixture = live_enrolled_session_fixture(
            "evidence_flow",
            "Live Evidence Provider",
            "provider-live-evidence-flow",
        )
        .await;
        let signed_report = signed_report_for_fixture(&fixture);
        let evidence_body = serde_json::to_string(&serde_json::json!({
            "evidence_type": "signed_report",
            "session_id": &fixture.session_id,
            "subject_id": "live-evidence-flow",
            "metadata": {"source": "live-http-contract"},
            "signed_report": signed_report
        }))
        .unwrap();
        let headers = fixture.session_headers();
        let submitted = send_request(
            fixture.app.clone(),
            Method::POST,
            &format!("/v1/sessions/{}/evidence-records", fixture.session_id),
            Some(&evidence_body),
            &headers,
        )
        .await;
        assert_eq!(submitted.status(), StatusCode::CREATED);
        let submitted = response_json(submitted).await;
        assert_eq!(submitted["duplicate"], false);
        assert_eq!(submitted["evidence"]["provider_id"], fixture.provider_id);
        assert_eq!(submitted["evidence"]["device_id"], fixture.device_id);
        assert_eq!(submitted["evidence"]["session_id"], fixture.session_id);
        assert_eq!(submitted["evidence"]["status"], "valid");
        assert_eq!(
            submitted["evidence"]["verification"]["signature_valid"],
            true
        );
        assert_eq!(
            submitted["evidence"]["verification"]["active_key_bound"],
            true
        );
        assert_eq!(
            submitted["evidence"]["verification"]["provider_bound"],
            true
        );
        assert_eq!(submitted["evidence"]["verification"]["device_bound"], true);
        assert_eq!(
            submitted["evidence"]["verification"]["fingerprint_bound"],
            true
        );
        assert_eq!(
            submitted["evidence"]["verification"]["expired_by_server"],
            false
        );
        assert!(
            submitted["evidence"]["object_key"]
                .as_str()
                .unwrap()
                .starts_with("evidence/")
        );
        let evidence_id = submitted["evidence"]["evidence_id"]
            .as_str()
            .unwrap()
            .to_string();
        let evidence_hash = submitted["evidence"]["evidence_hash"]
            .as_str()
            .unwrap()
            .to_string();

        let headers = fixture.session_headers();
        let duplicate = send_request(
            fixture.app.clone(),
            Method::POST,
            &format!("/v1/sessions/{}/evidence-records", fixture.session_id),
            Some(&evidence_body),
            &headers,
        )
        .await;
        assert_eq!(duplicate.status(), StatusCode::OK);
        let duplicate = response_json(duplicate).await;
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(duplicate["evidence"]["evidence_id"], evidence_id);
        assert_eq!(duplicate["evidence"]["evidence_hash"], evidence_hash);

        let listed = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/providers/{}/evidence-records", fixture.provider_id),
            None,
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(listed.status(), StatusCode::OK);
        let listed = response_json(listed).await;
        assert_eq!(listed["records"].as_array().unwrap().len(), 1);
        assert_eq!(listed["records"][0]["evidence_id"], evidence_id);

        let loaded = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/evidence-records/{evidence_id}"),
            None,
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(loaded.status(), StatusCode::OK);
        let loaded = response_json(loaded).await;
        assert_eq!(loaded["evidence_id"], evidence_id);
        assert_eq!(loaded["evidence_hash"], evidence_hash);

        let client = fixture.db.connect().await.unwrap();
        let persisted = client
            .query_one(
                "SELECT COUNT(*)::BIGINT AS evidence_count, (SELECT COUNT(*)::BIGINT FROM hardware_snapshots WHERE provider_id = $1 AND device_id = $2) AS snapshot_count FROM evidence_records WHERE provider_id = $1 AND device_id = $2 AND session_id = $3",
                &[&fixture.provider_id, &fixture.device_id, &fixture.session_id],
            )
            .await
            .unwrap();
        assert_eq!(persisted.get::<_, i64>("evidence_count"), 1);
        assert_eq!(persisted.get::<_, i64>("snapshot_count"), 1);

        fixture.cleanup().await;
    }

    #[tokio::test]
    async fn verification_sweep_fails_closed_without_proof_profile() {
        let response = send_request(
            test_app("postgres://unavailable"),
            Method::POST,
            "/v1/verification/sweep",
            Some("{}"),
            &[("authorization", "Bearer test-admin")],
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_error_envelope(&body, "invalid_request");
        assert_eq!(
            body["error"]["message"],
            "recurring verification proof profile is not configured"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_verification_sweep_persists_versioned_proof_profile() {
        let fixture = live_enrolled_session_fixture_with_config(
            "versioned_proof_profile",
            "Versioned Proof Profile Provider",
            "provider-versioned-proof-profile",
            true,
            |config| {
                config.verification_proof_profile =
                    Some(crate::config::VerificationProofProfileConfig {
                        profile_version: "poc-cuda-llm-v2".to_string(),
                        model_artifact_hash:
                            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                                .to_string(),
                        required_proofs: burd_protocol::PROOF_CAPABILITY_REQUIRED_PROOFS
                            .iter()
                            .map(|proof| (*proof).to_string())
                            .collect(),
                        min_tokens_per_second: 12.5,
                        max_ttft_ms: 1500,
                    });
            },
        )
        .await;
        submit_live_telemetry_batch(&fixture, 2, 1, "GPU-versioned-proof").await;

        let response = send_request(
            fixture.app.clone(),
            Method::POST,
            "/v1/verification/sweep",
            Some(r#"{"force":true,"reason":"versioned_profile_test"}"#),
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = response_json(response).await;
        assert_eq!(body["issued"].as_array().unwrap().len(), 1);
        let challenge_id = body["issued"][0]["challenge_id"]
            .as_str()
            .unwrap()
            .to_string();

        let response = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/challenges/{challenge_id}"),
            None,
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let challenge = response_json(response).await;
        assert_eq!(challenge["challenge"]["profile_version"], "poc-cuda-llm-v2");
        assert_eq!(
            challenge["challenge"]["model_artifact_hash"],
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(challenge["challenge"]["required_backend"], "cuda");
        assert_eq!(
            challenge["challenge"]["required_gpu_uuid"],
            "GPU-versioned-proof"
        );
        assert_eq!(challenge["challenge"]["min_tokens_per_second"], 12.5);
        assert_eq!(challenge["challenge"]["max_ttft_ms"], 1500);
        assert_eq!(
            challenge["challenge"]["required_proofs"],
            serde_json::json!(burd_protocol::PROOF_CAPABILITY_REQUIRED_PROOFS)
        );

        let client = fixture.db.connect().await.unwrap();
        let persisted = client
            .query_one(
                "SELECT pc.trigger_reason, pc.verification_policy_version, pc.model_artifact_hash, pc.required_proofs_json, pc.min_tokens_per_second, pc.max_ttft_ms, vs.status AS verification_status, vs.last_challenge_id FROM proof_challenges pc JOIN provider_verification_states vs ON vs.provider_id = pc.provider_id AND vs.device_id = pc.device_id WHERE pc.challenge_id = $1",
                &[&challenge_id],
            )
            .await
            .unwrap();
        assert_eq!(
            persisted
                .get::<_, Option<String>>("trigger_reason")
                .as_deref(),
            Some("versioned_profile_test")
        );
        assert_eq!(
            persisted
                .get::<_, Option<String>>("verification_policy_version")
                .as_deref(),
            Some(burd_protocol::VERIFICATION_POLICY_VERSION)
        );
        assert_eq!(
            persisted.get::<_, String>("model_artifact_hash"),
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        let required_proofs_json: String = persisted.get("required_proofs_json");
        let required_proofs: Vec<String> = serde_json::from_str(&required_proofs_json).unwrap();
        assert_eq!(
            required_proofs,
            burd_protocol::PROOF_CAPABILITY_REQUIRED_PROOFS
                .iter()
                .map(|proof| (*proof).to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(persisted.get::<_, f64>("min_tokens_per_second"), 12.5);
        assert_eq!(persisted.get::<_, i64>("max_ttft_ms"), 1500);
        assert_eq!(
            persisted.get::<_, String>("verification_status"),
            "verification_running"
        );
        assert_eq!(
            persisted
                .get::<_, Option<String>>("last_challenge_id")
                .as_deref(),
            Some(challenge_id.as_str())
        );

        fixture.cleanup().await;
    }
    #[tokio::test]
    #[ignore]
    async fn live_proof_challenge_http_flow_verifies_signed_response() {
        let fixture = live_enrolled_session_fixture(
            "proof_flow",
            "Live Proof Challenge Provider",
            "provider-live-proof-flow",
        )
        .await;
        let telemetry = submit_live_telemetry_batch(&fixture, 2, 1, "GPU-live-http").await;
        let challenge =
            issue_live_proof_challenge(&fixture, "live-proof-v1", "prompt_seed_live_http").await;

        let headers = fixture.session_headers();
        let next = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/sessions/{}/challenges/next", fixture.session_id),
            None,
            &headers,
        )
        .await;
        assert_eq!(next.status(), StatusCode::OK);
        let next = response_json(next).await;
        assert_eq!(next["challenge"]["challenge_id"], challenge.challenge_id);

        let signed = signed_proof_response_for_challenge(
            &fixture,
            &challenge,
            Some(telemetry.batch_hash.clone()),
        );
        let signed_body = serde_json::to_string(&signed).unwrap();
        let headers = fixture.session_headers();
        let submitted = send_request(
            fixture.app.clone(),
            Method::POST,
            &format!(
                "/v1/sessions/{}/challenges/{}/response",
                fixture.session_id, challenge.challenge_id
            ),
            Some(&signed_body),
            &headers,
        )
        .await;
        assert_eq!(submitted.status(), StatusCode::OK);
        let submitted = response_json(submitted).await;
        assert_eq!(submitted["challenge_id"], challenge.challenge_id);
        assert_eq!(submitted["status"], "verified");
        assert_eq!(submitted["response_hash"], signed.response_hash);
        assert_eq!(submitted["verification"]["signature_valid"], true);
        assert_eq!(submitted["verification"]["metrics_satisfied"], true);
        assert_eq!(submitted["verification"]["provider_bound"], true);
        assert_eq!(submitted["verification"]["device_bound"], true);
        assert_eq!(submitted["verification"]["session_bound"], true);
        assert_eq!(submitted["verification"]["fingerprint_bound"], true);
        assert!(
            submitted["verification"]["errors"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let loaded = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/challenges/{}", challenge.challenge_id),
            None,
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(loaded.status(), StatusCode::OK);
        let loaded = response_json(loaded).await;
        assert_eq!(loaded["status"], "verified");
        assert_eq!(loaded["response_hash"], signed.response_hash);
        assert_eq!(loaded["public_key_id"], fixture.public_key_id);
        assert_eq!(loaded["verification"]["response_hash_valid"], true);

        let client = fixture.db.connect().await.unwrap();
        let persisted = client
            .query_one(
                "SELECT pc.status, pc.response_hash, pc.public_key_id, pc.response_json, vs.status AS verification_status, vs.success_count FROM proof_challenges pc LEFT JOIN provider_verification_states vs ON vs.last_verified_challenge_id = pc.challenge_id WHERE pc.challenge_id = $1",
                &[&challenge.challenge_id],
            )
            .await
            .unwrap();
        assert_eq!(persisted.get::<_, String>("status"), "verified");
        assert_eq!(
            persisted.get::<_, Option<String>>("response_hash"),
            Some(signed.response_hash)
        );
        assert_eq!(
            persisted.get::<_, Option<String>>("public_key_id"),
            Some(fixture.public_key_id.clone())
        );
        let response_json: String = persisted.get::<_, Option<String>>("response_json").unwrap();
        let persisted_response: burd_protocol::SignedProofCapabilityResponse =
            serde_json::from_str(&response_json).unwrap();
        assert_eq!(
            persisted_response.payload.telemetry_window_hash,
            Some(telemetry.batch_hash)
        );
        assert_eq!(
            persisted.get::<_, Option<String>>("verification_status"),
            Some("verified".to_string())
        );
        assert_eq!(persisted.get::<_, Option<i32>>("success_count"), Some(1));

        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn live_proof_challenge_rejects_unregistered_telemetry_window_hash() {
        let fixture = live_enrolled_session_fixture(
            "proof_missing_telemetry",
            "Live Proof Missing Telemetry Provider",
            "provider-live-proof-missing-telemetry",
        )
        .await;
        let challenge = issue_live_proof_challenge(
            &fixture,
            "live-proof-missing-telemetry-v1",
            "prompt_seed_missing_telemetry",
        )
        .await;
        let signed = signed_proof_response_for_challenge(
            &fixture,
            &challenge,
            Some("sha256:not-a-verified-telemetry-window".to_string()),
        );
        let signed_body = serde_json::to_string(&signed).unwrap();
        let headers = fixture.session_headers();
        let submitted = send_request(
            fixture.app.clone(),
            Method::POST,
            &format!(
                "/v1/sessions/{}/challenges/{}/response",
                fixture.session_id, challenge.challenge_id
            ),
            Some(&signed_body),
            &headers,
        )
        .await;
        assert_eq!(submitted.status(), StatusCode::OK);
        let submitted = response_json(submitted).await;
        assert_eq!(submitted["challenge_id"], challenge.challenge_id);
        assert_eq!(submitted["status"], "failed");
        assert_eq!(submitted["verification"]["metrics_satisfied"], false);
        let errors = submitted["verification"]["errors"].as_array().unwrap();
        assert!(errors.iter().any(|error| {
            error
                .as_str()
                .unwrap()
                .contains("telemetry window hash is not a verified telemetry batch")
        }));

        let loaded = send_request(
            fixture.app.clone(),
            Method::GET,
            &format!("/v1/challenges/{}", challenge.challenge_id),
            None,
            &[("authorization", "Bearer test-admin")],
        )
        .await;
        assert_eq!(loaded.status(), StatusCode::OK);
        let loaded = response_json(loaded).await;
        assert_eq!(loaded["status"], "failed");
        assert_eq!(loaded["verification"]["metrics_satisfied"], false);

        fixture.cleanup().await;
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

    #[test]
    fn artifact_object_paths_reject_escape_components() {
        let root = std::env::temp_dir().join(format!(
            "burd-artifact-path-test-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        assert!(object_path(&root, "jobs/job_1/input.bin").is_ok());
        assert!(object_path(&root, "../outside.bin").is_err());
        assert!(object_path(&root, "/absolute.bin").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn upload_writer_streams_exact_size_and_digest() {
        let root = FilePath::new("target/test-control-objects").join(format!(
            "burd-artifact-upload-test-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        let temporary = root.join("upload.tmp");
        let payload = Bytes::from_static(b"artifact-bytes");
        let digest = format!("sha256:{}", sha256_hex(&payload));
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender.try_send(payload.clone()).unwrap();
        drop(sender);

        let written = write_upload_stream(
            &temporary,
            receiver,
            payload.len() as u64,
            payload.len() as u64,
            &digest,
        )
        .unwrap();

        assert_eq!(written.sha256, digest);
        assert_eq!(written.size_bytes, payload.len() as u64);
        assert_eq!(fs::read(&temporary).unwrap(), payload);
        fs::remove_dir_all(root).unwrap();
    }
}
