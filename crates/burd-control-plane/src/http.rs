use crate::config::ControlPlaneConfig;
use crate::db::{CreateProviderCommand, CreateProviderOutcome, Database, ProviderRecord};
use crate::enrollment::EnrollmentError;
use crate::error::{ApiError, ErrorCode};
use crate::openapi;
use crate::rate_limit::RateLimiter;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use burd_protocol::{
    EnrollmentProofRequest, KeyRotationProofRequest, StartEnrollmentRequest,
    StartKeyRotationRequest, hash_canonical, sha256_hex,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug)]
pub struct AppState {
    pub config: ControlPlaneConfig,
    pub db: Database,
    pub rate_limiter: RateLimiter,
}

impl AppState {
    pub fn new(config: ControlPlaneConfig, db: Database) -> Self {
        let rate_limiter = RateLimiter::per_minute(config.rate_limit_per_minute);
        Self {
            config,
            db,
            rate_limiter,
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
            rate_limit_per_minute: 120,
            admin_token_hash: sha256_hex(b"test-admin"),
            enrollment_token_ttl_seconds: 600,
            enrollment_proof_ttl_seconds: 300,
            device_credential_ttl_seconds: 900,
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
            &["0001".to_string(), "0002".to_string()],
            &["0001", "0002"]
        ));
        assert!(!migrations_are_current(
            &["0001".to_string()],
            &["0001", "0002"]
        ));
        assert!(!migrations_are_current(
            &[
                "0001".to_string(),
                "0002".to_string(),
                "unexpected".to_string()
            ],
            &["0001", "0002"]
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
