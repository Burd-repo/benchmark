use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    IdempotencyConflict,
    RateLimited,
    Expired,
    Revoked,
    SignatureInvalid,
    NonceReused,
    PolicyBlocked,
    DatabaseUnavailable,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::RateLimited => "rate_limited",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::SignatureInvalid => "signature_invalid",
            Self::NonceReused => "nonce_reused",
            Self::PolicyBlocked => "policy_blocked",
            Self::DatabaseUnavailable => "database_unavailable",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: ErrorCode,
    pub message: String,
    pub request_id: String,
    pub retry_after_seconds: Option<u64>,
    pub details: serde_json::Value,
}

impl ApiError {
    pub fn new(
        status: StatusCode,
        code: ErrorCode,
        message: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            request_id: request_id.into(),
            retry_after_seconds: None,
            details: serde_json::json!({}),
        }
    }

    pub fn invalid_request(message: impl Into<String>, request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            message,
            request_id,
        )
    }

    pub fn database(error: impl std::fmt::Display, request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::DatabaseUnavailable,
            format!("database unavailable: {error}"),
            request_id,
        )
    }

    pub fn idempotency_conflict(request_id: impl Into<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::IdempotencyConflict,
            "idempotency key was reused with a different request body",
            request_id,
        )
    }

    pub fn rate_limited(request_id: impl Into<String>, retry_after_seconds: u64) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "rate limit exceeded",
            request_id,
        );
        error.retry_after_seconds = Some(retry_after_seconds);
        error
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": {
                "code": self.code.as_str(),
                "message": self.message,
                "request_id": self.request_id,
                "retry_after_seconds": self.retry_after_seconds,
                "details": self.details,
            }
        });
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(
            ErrorCode::IdempotencyConflict.as_str(),
            "idempotency_conflict"
        );
        assert_eq!(
            ErrorCode::DatabaseUnavailable.as_str(),
            "database_unavailable"
        );
    }
}
