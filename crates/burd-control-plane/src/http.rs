use crate::config::ControlPlaneConfig;
use crate::db::{Database, NewAuditEvent, ProviderRecord};
use crate::error::{ApiError, ErrorCode};
use crate::openapi;
use crate::rate_limit::RateLimiter;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use burd_protocol::hash_canonical;
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
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let request_hash = hash_canonical(&payload)
        .map_err(|error| ApiError::invalid_request(error, request_id.clone()))?;
    let scope = "POST /v1/providers";

    if let Some(record) = state
        .db
        .get_idempotency_record(scope, &idempotency_key)
        .await
        .map_err(|error| ApiError::database(error, request_id.clone()))?
    {
        if record.request_hash != request_hash {
            return Err(ApiError::idempotency_conflict(request_id));
        }
        let value = serde_json::from_str::<serde_json::Value>(&record.response_json)
            .map_err(|error| ApiError::invalid_request(error.to_string(), request_id.clone()))?;
        let status = StatusCode::from_u16(record.status_code).unwrap_or(StatusCode::OK);
        return Ok((status, Json(value)).into_response());
    }

    let provider = state
        .db
        .create_provider(payload.user_id, payload.display_name)
        .await
        .map_err(|error| ApiError::database(error, request_id.clone()))?;
    let audit_event_id = state
        .db
        .insert_audit_event(NewAuditEvent {
            request_id: &request_id,
            actor_type: "system",
            actor_id: None,
            entity_type: "provider",
            entity_id: &provider.provider_id,
            event_type: "provider.created",
            idempotency_key: Some(idempotency_key.clone()),
            summary: "provider registry record created",
            metadata_json: "{}",
        })
        .await
        .map_err(|error| ApiError::database(error, request_id.clone()))?;

    let response = serde_json::json!(ProviderEnvelope {
        request_id: request_id.clone(),
        audit_event_id: Some(audit_event_id),
        provider,
    });
    let response_json = serde_json::to_string(&response)
        .map_err(|error| ApiError::invalid_request(error.to_string(), request_id.clone()))?;
    state
        .db
        .put_idempotency_record(
            scope,
            &idempotency_key,
            &request_hash,
            StatusCode::CREATED.as_u16(),
            &response_json,
        )
        .await
        .map_err(|error| ApiError::database(error, request_id.clone()))?;

    Ok((StatusCode::CREATED, Json(response)).into_response())
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

fn new_request_id() -> String {
    format!("req_{}", Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_endpoint_does_not_require_database_connection() {
        let config = ControlPlaneConfig {
            environment: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 0,
            database_url: "postgres://localhost/unavailable".to_string(),
            database_schema: None,
            rate_limit_per_minute: 120,
        };
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
}
