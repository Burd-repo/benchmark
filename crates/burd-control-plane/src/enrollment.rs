use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use burd_protocol::{
    DeviceCredentialResponse, DeviceRecord, DeviceRevocationResponse, EnrollmentProofRequest,
    EnrollmentProofResponse, IssueEnrollmentTokenResponse, KEY_ALGORITHM, KeyRotationProofRequest,
    KeyRotationProofResponse, StartEnrollmentRequest, StartEnrollmentResponse,
    StartKeyRotationRequest, StartKeyRotationResponse, enrollment_proof_message, hash_canonical,
    key_rotation_proof_message, random_token, sha256_hex, validate_public_key, verify_message,
};
use chrono::{DateTime, Duration, Utc};
use std::fmt;
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

#[derive(Debug)]
pub enum EnrollmentError {
    Database(DbError),
    NotFound(String),
    Invalid(String),
    Unauthorized,
    Expired,
    Revoked,
    NonceReused,
    Conflict(String),
    SignatureInvalid,
}

impl fmt::Display for EnrollmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "{error}"),
            Self::NotFound(message) | Self::Invalid(message) | Self::Conflict(message) => {
                formatter.write_str(message)
            }
            Self::Unauthorized => formatter.write_str("device credential is invalid or expired"),
            Self::Expired => formatter.write_str("enrollment proof has expired"),
            Self::Revoked => formatter.write_str("device or enrollment has been revoked"),
            Self::NonceReused => formatter.write_str("nonce was already used"),
            Self::SignatureInvalid => formatter.write_str("Ed25519 signature is invalid"),
        }
    }
}

impl std::error::Error for EnrollmentError {}

impl From<DbError> for EnrollmentError {
    fn from(error: DbError) -> Self {
        Self::Database(error)
    }
}

impl From<tokio_postgres::Error> for EnrollmentError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Database(DbError::from(error))
    }
}

#[derive(Debug)]
pub(crate) struct DeviceAuth {
    pub(crate) provider_id: String,
    pub(crate) device_id: String,
    pub(crate) public_key_id: String,
}

impl Database {
    pub async fn issue_enrollment_token(
        &self,
        provider_id: &str,
        request_id: &str,
        ttl_seconds: u32,
    ) -> Result<IssueEnrollmentTokenResponse, EnrollmentError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let provider = transaction
            .query_opt(
                "SELECT status FROM providers WHERE provider_id = $1 FOR UPDATE",
                &[&provider_id],
            )
            .await?
            .ok_or_else(|| EnrollmentError::NotFound("provider not found".to_string()))?;
        let provider_status: String = provider.get("status");
        if matches!(provider_status.as_str(), "blocked" | "quarantined") {
            return Err(EnrollmentError::Revoked);
        }

        let now = Utc::now();
        let issued_at = now.to_rfc3339();
        let expires_at = (now + Duration::seconds(i64::from(ttl_seconds))).to_rfc3339();
        let token = random_token("burd_enroll").map_err(EnrollmentError::Invalid)?;
        let token_hash = sha256_hex(token.as_bytes());
        let token_id = format!("enrollment_token_{}", Uuid::new_v4());
        transaction
            .execute(
                "UPDATE enrollment_tokens SET status = 'revoked', revoked_at = $1 WHERE provider_id = $2 AND status = 'issued'",
                &[&issued_at, &provider_id],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO enrollment_tokens (enrollment_token_id, provider_id, token_hash, status, issued_at, expires_at) VALUES ($1, $2, $3, 'issued', $4, $5)",
                &[&token_id, &provider_id, &token_hash, &issued_at, &expires_at],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "provider",
                entity_id: provider_id,
                event_type: "enrollment_token.issued",
                idempotency_key: None,
                summary: "one-time provider enrollment token issued",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;

        Ok(IssueEnrollmentTokenResponse {
            request_id: request_id.to_string(),
            enrollment_token: token,
            expires_at,
            max_uses: 1,
        })
    }

    pub async fn start_enrollment(
        &self,
        request_id: &str,
        request: &StartEnrollmentRequest,
        ttl_seconds: u32,
    ) -> Result<StartEnrollmentResponse, EnrollmentError> {
        validate_enrollment_request(request)?;
        let token_hash = sha256_hex(request.enrollment_token.as_bytes());
        let request_hash = hash_canonical(request).map_err(EnrollmentError::Invalid)?;
        let registration_payload_json = serde_json::to_string(&request.registration_payload)
            .map_err(|error| EnrollmentError::Invalid(error.to_string()))?;
        let registration_payload_hash =
            hash_canonical(&request.registration_payload).map_err(EnrollmentError::Invalid)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let token = transaction
            .query_opt(
                "SELECT enrollment_token_id, provider_id, status, expires_at FROM enrollment_tokens WHERE token_hash = $1 FOR UPDATE",
                &[&token_hash],
            )
            .await?
            .ok_or(EnrollmentError::Unauthorized)?;
        let token_id: String = token.get("enrollment_token_id");
        let provider_id: String = token.get("provider_id");
        let token_status: String = token.get("status");
        let token_expires_at: String = token.get("expires_at");

        if token_status == "used" {
            let existing = transaction
                .query_opt(
                    "SELECT enrollment_id, nonce, expires_at, request_hash, status FROM device_enrollments WHERE enrollment_token_id = $1",
                    &[&token_id],
                )
                .await?;
            if let Some(existing) = existing {
                let existing_hash: String = existing.get("request_hash");
                let status: String = existing.get("status");
                if existing_hash == request_hash && status == "pending_proof" {
                    let response = StartEnrollmentResponse {
                        request_id: request_id.to_string(),
                        enrollment_id: existing.get("enrollment_id"),
                        provider_id,
                        nonce: existing.get("nonce"),
                        expires_at: existing.get("expires_at"),
                    };
                    transaction.commit().await?;
                    return Ok(response);
                }
            }
            return Err(EnrollmentError::NonceReused);
        }
        if token_status == "revoked" {
            return Err(EnrollmentError::Revoked);
        }
        if token_status != "issued" {
            return Err(EnrollmentError::Unauthorized);
        }
        if is_expired(&token_expires_at)? {
            transaction
                .execute(
                    "UPDATE enrollment_tokens SET status = 'expired' WHERE enrollment_token_id = $1",
                    &[&token_id],
                )
                .await?;
            transaction.commit().await?;
            return Err(EnrollmentError::Expired);
        }

        let now = Utc::now();
        let issued_at = now.to_rfc3339();
        let expires_at = (now + Duration::seconds(i64::from(ttl_seconds))).to_rfc3339();
        let enrollment_id = format!("enrollment_{}", Uuid::new_v4());
        let nonce = random_token("burd_nonce").map_err(EnrollmentError::Invalid)?;
        transaction
            .execute(
                "UPDATE enrollment_tokens SET status = 'used', used_at = $1 WHERE enrollment_token_id = $2",
                &[&issued_at, &token_id],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO device_enrollments (enrollment_id, enrollment_token_id, provider_id, local_provider_id, machine_id, public_key, key_algorithm, registration_payload_json, registration_payload_hash, hardware_fingerprint, agent_version, benchmark_version, nonce, request_hash, status, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, 'pending_proof', $15, $16)",
                &[
                    &enrollment_id,
                    &token_id,
                    &provider_id,
                    &request.local_provider_id,
                    &request.machine_id,
                    &request.public_key,
                    &request.key_algorithm,
                    &registration_payload_json,
                    &registration_payload_hash,
                    &request.hardware_fingerprint,
                    &request.agent_version,
                    &request.benchmark_version,
                    &nonce,
                    &request_hash,
                    &issued_at,
                    &expires_at,
                ],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "enrollment_token",
                actor_id: Some(token_id),
                entity_type: "enrollment",
                entity_id: &enrollment_id,
                event_type: "enrollment.started",
                idempotency_key: None,
                summary: "device enrollment nonce issued",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;

        Ok(StartEnrollmentResponse {
            request_id: request_id.to_string(),
            enrollment_id,
            provider_id,
            nonce,
            expires_at,
        })
    }

    pub async fn complete_enrollment(
        &self,
        enrollment_id: &str,
        request_id: &str,
        request: &EnrollmentProofRequest,
        credential_ttl_seconds: u32,
    ) -> Result<EnrollmentProofResponse, EnrollmentError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_opt(
                "SELECT provider_id, machine_id, public_key, hardware_fingerprint, nonce, status, expires_at FROM device_enrollments WHERE enrollment_id = $1 FOR UPDATE",
                &[&enrollment_id],
            )
            .await?
            .ok_or_else(|| EnrollmentError::NotFound("enrollment not found".to_string()))?;
        let provider_id: String = row.get("provider_id");
        let machine_id: String = row.get("machine_id");
        let public_key: String = row.get("public_key");
        let hardware_fingerprint: String = row.get("hardware_fingerprint");
        let nonce: String = row.get("nonce");
        let status: String = row.get("status");
        let expires_at: String = row.get("expires_at");

        if status == "completed" {
            return Err(EnrollmentError::NonceReused);
        }
        if status == "revoked" {
            return Err(EnrollmentError::Revoked);
        }
        if status != "pending_proof" {
            return Err(EnrollmentError::Conflict(format!(
                "enrollment cannot be completed from status {status}"
            )));
        }
        if is_expired(&expires_at)? {
            transaction
                .execute(
                    "UPDATE device_enrollments SET status = 'expired' WHERE enrollment_id = $1",
                    &[&enrollment_id],
                )
                .await?;
            transaction.commit().await?;
            return Err(EnrollmentError::Expired);
        }
        let binding_error = if request.nonce != nonce {
            Some("nonce mismatch")
        } else if request.public_key != public_key {
            Some("public key does not match enrollment")
        } else if request.hardware_fingerprint != hardware_fingerprint {
            Some("hardware fingerprint does not match enrollment")
        } else {
            None
        };
        if let Some(message) = binding_error {
            record_failed_proof(
                &transaction,
                enrollment_id,
                request_id,
                "enrollment.proof_rejected",
            )
            .await?;
            transaction.commit().await?;
            return Err(EnrollmentError::Invalid(message.to_string()));
        }

        let message = enrollment_proof_message(
            enrollment_id,
            &provider_id,
            &machine_id,
            &nonce,
            &public_key,
            &hardware_fingerprint,
            &expires_at,
        )
        .map_err(EnrollmentError::Invalid)?;
        let valid =
            verify_message(&public_key, message.as_bytes(), &request.signature).unwrap_or(false);

        if !valid {
            record_failed_proof(
                &transaction,
                enrollment_id,
                request_id,
                "enrollment.proof_rejected",
            )
            .await?;
            transaction.commit().await?;
            return Err(EnrollmentError::SignatureInvalid);
        }
        if transaction
            .query_opt(
                "SELECT device_id FROM devices WHERE provider_id = $1 AND machine_id = $2",
                &[&provider_id, &machine_id],
            )
            .await?
            .is_some()
        {
            return Err(EnrollmentError::Conflict(
                "machine is already enrolled for this provider".to_string(),
            ));
        }
        if transaction
            .query_opt(
                "SELECT public_key_id FROM provider_public_keys WHERE public_key = $1 AND status = 'active'",
                &[&public_key],
            )
            .await?
            .is_some()
        {
            return Err(EnrollmentError::Conflict(
                "public key is already active on another device".to_string(),
            ));
        }

        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let credential_expires_at =
            (now + Duration::seconds(i64::from(credential_ttl_seconds))).to_rfc3339();
        let device_id = format!("device_{}", Uuid::new_v4());
        let identity_id = format!("identity_{}", Uuid::new_v4());
        let public_key_id = format!("public_key_{}", Uuid::new_v4());
        let credential_id = format!("credential_{}", Uuid::new_v4());
        let credential = random_token("burd_device").map_err(EnrollmentError::Invalid)?;
        let credential_hash = sha256_hex(credential.as_bytes());
        transaction
            .execute(
                "INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ($1, $2, $3, 'active', $4, $4)",
                &[&device_id, &provider_id, &machine_id, &now_text],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO provider_identities (identity_id, provider_id, device_id, status, created_at) VALUES ($1, $2, $3, 'active', $4)",
                &[&identity_id, &provider_id, &device_id, &now_text],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at) VALUES ($1, $2, $3, $4, $5, 'active', $6)",
                &[
                    &public_key_id,
                    &provider_id,
                    &device_id,
                    &public_key,
                    &KEY_ALGORITHM,
                    &now_text,
                ],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO device_credentials (credential_id, provider_id, device_id, credential_hash, status, issued_at, expires_at) VALUES ($1, $2, $3, $4, 'active', $5, $6)",
                &[
                    &credential_id,
                    &provider_id,
                    &device_id,
                    &credential_hash,
                    &now_text,
                    &credential_expires_at,
                ],
            )
            .await?;
        transaction
            .execute(
                "UPDATE device_enrollments SET status = 'completed', proof_attempts = proof_attempts + 1, nonce_used_at = $1, completed_at = $1, device_id = $2 WHERE enrollment_id = $3",
                &[&now_text, &device_id, &enrollment_id],
            )
            .await?;
        transaction
            .execute(
                "UPDATE providers SET status = 'pending_verification', updated_at = $1 WHERE provider_id = $2",
                &[&now_text, &provider_id],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_key",
                actor_id: Some(public_key_id.clone()),
                entity_type: "device",
                entity_id: &device_id,
                event_type: "enrollment.completed",
                idempotency_key: None,
                summary: "device enrolled after Ed25519 possession proof",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;

        Ok(EnrollmentProofResponse {
            request_id: request_id.to_string(),
            provider_id,
            device_id,
            public_key_id,
            credential,
            credential_expires_at,
            status: "pending_verification".to_string(),
        })
    }

    pub async fn list_provider_devices(
        &self,
        provider_id: &str,
    ) -> Result<Vec<DeviceRecord>, EnrollmentError> {
        let client = self.connect().await?;
        if client
            .query_opt(
                "SELECT provider_id FROM providers WHERE provider_id = $1",
                &[&provider_id],
            )
            .await?
            .is_none()
        {
            return Err(EnrollmentError::NotFound("provider not found".to_string()));
        }
        let rows = client
            .query(
                "SELECT d.device_id, d.provider_id, d.machine_id, d.status, d.created_at, d.updated_at, k.public_key_id AS active_public_key_id FROM devices d LEFT JOIN provider_public_keys k ON k.device_id = d.device_id AND k.status = 'active' WHERE d.provider_id = $1 ORDER BY d.created_at",
                &[&provider_id],
            )
            .await?;
        Ok(rows.into_iter().map(device_from_row).collect())
    }

    pub async fn refresh_device_credential(
        &self,
        device_id: &str,
        credential: &str,
        request_id: &str,
        ttl_seconds: u32,
    ) -> Result<DeviceCredentialResponse, EnrollmentError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let auth = authenticate_device(&transaction, device_id, credential).await?;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let expires_at = (now + Duration::seconds(i64::from(ttl_seconds))).to_rfc3339();
        let next_credential = random_token("burd_device").map_err(EnrollmentError::Invalid)?;
        let next_hash = sha256_hex(next_credential.as_bytes());
        let credential_id = format!("credential_{}", Uuid::new_v4());
        transaction
            .execute(
                "UPDATE device_credentials SET status = 'revoked', revoked_at = $1 WHERE device_id = $2 AND status = 'active'",
                &[&now_text, &device_id],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO device_credentials (credential_id, provider_id, device_id, credential_hash, status, issued_at, expires_at) VALUES ($1, $2, $3, $4, 'active', $5, $6)",
                &[
                    &credential_id,
                    &auth.provider_id,
                    &auth.device_id,
                    &next_hash,
                    &now_text,
                    &expires_at,
                ],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_credential",
                actor_id: Some(credential_id),
                entity_type: "device",
                entity_id: device_id,
                event_type: "device_credential.rotated",
                idempotency_key: None,
                summary: "short-lived device credential rotated",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(DeviceCredentialResponse {
            request_id: request_id.to_string(),
            provider_id: auth.provider_id,
            device_id: auth.device_id,
            credential: next_credential,
            credential_expires_at: expires_at,
        })
    }

    pub async fn start_key_rotation(
        &self,
        device_id: &str,
        credential: &str,
        request_id: &str,
        request: &StartKeyRotationRequest,
        ttl_seconds: u32,
    ) -> Result<StartKeyRotationResponse, EnrollmentError> {
        validate_key_request(&request.new_public_key, &request.key_algorithm)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let auth = authenticate_device(&transaction, device_id, credential).await?;
        if transaction
            .query_opt(
                "SELECT public_key_id FROM provider_public_keys WHERE public_key = $1 AND status = 'active'",
                &[&request.new_public_key],
            )
            .await?
            .is_some()
        {
            return Err(EnrollmentError::Conflict(
                "new public key is already active".to_string(),
            ));
        }
        let now = Utc::now();
        let issued_at = now.to_rfc3339();
        let expires_at = (now + Duration::seconds(i64::from(ttl_seconds))).to_rfc3339();
        let rotation_id = format!("rotation_{}", Uuid::new_v4());
        let nonce = random_token("burd_nonce").map_err(EnrollmentError::Invalid)?;
        transaction
            .execute(
                "UPDATE key_rotation_challenges SET status = 'revoked' WHERE device_id = $1 AND status = 'pending_proof'",
                &[&device_id],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO key_rotation_challenges (rotation_id, provider_id, device_id, current_public_key_id, new_public_key, key_algorithm, nonce, status, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending_proof', $8, $9)",
                &[
                    &rotation_id,
                    &auth.provider_id,
                    &auth.device_id,
                    &auth.public_key_id,
                    &request.new_public_key,
                    &request.key_algorithm,
                    &nonce,
                    &issued_at,
                    &expires_at,
                ],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_credential",
                actor_id: None,
                entity_type: "device",
                entity_id: device_id,
                event_type: "key_rotation.started",
                idempotency_key: None,
                summary: "key rotation nonce issued",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(StartKeyRotationResponse {
            request_id: request_id.to_string(),
            rotation_id,
            provider_id: auth.provider_id,
            device_id: auth.device_id,
            current_public_key_id: auth.public_key_id,
            nonce,
            expires_at,
        })
    }

    pub async fn complete_key_rotation(
        &self,
        device_id: &str,
        rotation_id: &str,
        credential: &str,
        request_id: &str,
        request: &KeyRotationProofRequest,
    ) -> Result<KeyRotationProofResponse, EnrollmentError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let auth = authenticate_device(&transaction, device_id, credential).await?;
        let row = transaction
            .query_opt(
                "SELECT provider_id, current_public_key_id, new_public_key, nonce, status, expires_at FROM key_rotation_challenges WHERE rotation_id = $1 AND device_id = $2 FOR UPDATE",
                &[&rotation_id, &device_id],
            )
            .await?
            .ok_or_else(|| EnrollmentError::NotFound("key rotation not found".to_string()))?;
        let provider_id: String = row.get("provider_id");
        let current_public_key_id: String = row.get("current_public_key_id");
        let new_public_key: String = row.get("new_public_key");
        let nonce: String = row.get("nonce");
        let status: String = row.get("status");
        let expires_at: String = row.get("expires_at");
        if status == "completed" {
            return Err(EnrollmentError::NonceReused);
        }
        if status == "revoked" {
            return Err(EnrollmentError::Revoked);
        }
        if is_expired(&expires_at)? {
            transaction
                .execute(
                    "UPDATE key_rotation_challenges SET status = 'expired' WHERE rotation_id = $1",
                    &[&rotation_id],
                )
                .await?;
            transaction.commit().await?;
            return Err(EnrollmentError::Expired);
        }
        if request.nonce != nonce || request.new_public_key != new_public_key {
            record_failed_rotation_proof(&transaction, rotation_id, request_id).await?;
            transaction.commit().await?;
            return Err(EnrollmentError::Invalid(
                "key rotation proof does not match challenge".to_string(),
            ));
        }
        if current_public_key_id != auth.public_key_id || provider_id != auth.provider_id {
            return Err(EnrollmentError::Conflict(
                "active device key changed during rotation".to_string(),
            ));
        }
        let message = key_rotation_proof_message(
            rotation_id,
            &provider_id,
            device_id,
            &nonce,
            &current_public_key_id,
            &new_public_key,
            &expires_at,
        )
        .map_err(EnrollmentError::Invalid)?;
        let valid = verify_message(&new_public_key, message.as_bytes(), &request.signature)
            .unwrap_or(false);
        if !valid {
            record_failed_rotation_proof(&transaction, rotation_id, request_id).await?;
            transaction.commit().await?;
            return Err(EnrollmentError::SignatureInvalid);
        }

        let now = Utc::now().to_rfc3339();
        let public_key_id = format!("public_key_{}", Uuid::new_v4());
        transaction
            .execute(
                "UPDATE provider_public_keys SET status = 'revoked', revoked_at = $1 WHERE public_key_id = $2",
                &[&now, &current_public_key_id],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at) VALUES ($1, $2, $3, $4, $5, 'active', $6)",
                &[
                    &public_key_id,
                    &provider_id,
                    &device_id,
                    &new_public_key,
                    &KEY_ALGORITHM,
                    &now,
                ],
            )
            .await?;
        transaction
            .execute(
                "UPDATE key_rotation_challenges SET status = 'completed', proof_attempts = proof_attempts + 1, completed_at = $1 WHERE rotation_id = $2",
                &[&now, &rotation_id],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_key",
                actor_id: Some(public_key_id.clone()),
                entity_type: "device",
                entity_id: device_id,
                event_type: "key_rotation.completed",
                idempotency_key: None,
                summary: "active Ed25519 device key rotated",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(KeyRotationProofResponse {
            request_id: request_id.to_string(),
            provider_id,
            device_id: device_id.to_string(),
            public_key_id,
            status: "active".to_string(),
        })
    }

    pub async fn revoke_device(
        &self,
        device_id: &str,
        request_id: &str,
    ) -> Result<DeviceRevocationResponse, EnrollmentError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_opt(
                "SELECT provider_id, status FROM devices WHERE device_id = $1 FOR UPDATE",
                &[&device_id],
            )
            .await?
            .ok_or_else(|| EnrollmentError::NotFound("device not found".to_string()))?;
        let provider_id: String = row.get("provider_id");
        let now = Utc::now().to_rfc3339();
        transaction
            .execute(
                "UPDATE devices SET status = 'revoked', updated_at = $1 WHERE device_id = $2",
                &[&now, &device_id],
            )
            .await?;
        transaction
            .execute(
                "UPDATE provider_public_keys SET status = 'revoked', revoked_at = COALESCE(revoked_at, $1) WHERE device_id = $2 AND status = 'active'",
                &[&now, &device_id],
            )
            .await?;
        transaction
            .execute(
                "UPDATE provider_identities SET status = 'revoked', revoked_at = COALESCE(revoked_at, $1) WHERE device_id = $2 AND status = 'active'",
                &[&now, &device_id],
            )
            .await?;
        transaction
            .execute(
                "UPDATE device_credentials SET status = 'revoked', revoked_at = COALESCE(revoked_at, $1) WHERE device_id = $2 AND status = 'active'",
                &[&now, &device_id],
            )
            .await?;
        transaction
            .execute(
                "UPDATE key_rotation_challenges SET status = 'revoked' WHERE device_id = $1 AND status = 'pending_proof'",
                &[&device_id],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "device",
                entity_id: device_id,
                event_type: "device.revoked",
                idempotency_key: None,
                summary: "device, keys, and credentials revoked",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(DeviceRevocationResponse {
            request_id: request_id.to_string(),
            provider_id,
            device_id: device_id.to_string(),
            status: "revoked".to_string(),
            revoked_at: now,
        })
    }
}

pub(crate) async fn authenticate_device(
    transaction: &Transaction<'_>,
    device_id: &str,
    credential: &str,
) -> Result<DeviceAuth, EnrollmentError> {
    let credential_hash = sha256_hex(credential.as_bytes());
    let row = transaction
        .query_opt(
            "SELECT c.provider_id, c.device_id, c.status AS credential_status, c.expires_at, d.status AS device_status, k.public_key_id FROM device_credentials c JOIN devices d ON d.device_id = c.device_id LEFT JOIN provider_public_keys k ON k.device_id = d.device_id AND k.status = 'active' WHERE c.device_id = $1 AND c.credential_hash = $2 FOR UPDATE OF c, d",
            &[&device_id, &credential_hash],
        )
        .await?
        .ok_or(EnrollmentError::Unauthorized)?;
    let credential_status: String = row.get("credential_status");
    let device_status: String = row.get("device_status");
    let expires_at: String = row.get("expires_at");
    let public_key_id: Option<String> = row.get("public_key_id");
    if credential_status != "active" || device_status != "active" {
        return Err(EnrollmentError::Revoked);
    }
    let public_key_id = public_key_id.ok_or(EnrollmentError::Revoked)?;
    if is_expired(&expires_at)? {
        transaction
            .execute(
                "UPDATE device_credentials SET status = 'expired' WHERE device_id = $1 AND credential_hash = $2",
                &[&device_id, &credential_hash],
            )
            .await?;
        return Err(EnrollmentError::Unauthorized);
    }
    transaction
        .execute(
            "UPDATE device_credentials SET last_used_at = $1 WHERE device_id = $2 AND credential_hash = $3",
            &[&Utc::now().to_rfc3339(), &device_id, &credential_hash],
        )
        .await?;
    Ok(DeviceAuth {
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        public_key_id,
    })
}

async fn record_failed_proof(
    transaction: &Transaction<'_>,
    enrollment_id: &str,
    request_id: &str,
    event_type: &str,
) -> Result<(), EnrollmentError> {
    transaction
        .execute(
            "UPDATE device_enrollments SET proof_attempts = proof_attempts + 1 WHERE enrollment_id = $1",
            &[&enrollment_id],
        )
        .await?;
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id,
            actor_type: "device_key",
            actor_id: None,
            entity_type: "enrollment",
            entity_id: enrollment_id,
            event_type,
            idempotency_key: None,
            summary: "Ed25519 enrollment proof rejected",
            metadata_json: "{}",
        },
    )
    .await?;
    Ok(())
}

async fn record_failed_rotation_proof(
    transaction: &Transaction<'_>,
    rotation_id: &str,
    request_id: &str,
) -> Result<(), EnrollmentError> {
    transaction
        .execute(
            "UPDATE key_rotation_challenges SET proof_attempts = proof_attempts + 1 WHERE rotation_id = $1",
            &[&rotation_id],
        )
        .await?;
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id,
            actor_type: "device_key",
            actor_id: None,
            entity_type: "key_rotation",
            entity_id: rotation_id,
            event_type: "key_rotation.proof_rejected",
            idempotency_key: None,
            summary: "Ed25519 key rotation proof rejected",
            metadata_json: "{}",
        },
    )
    .await?;
    Ok(())
}

fn validate_enrollment_request(request: &StartEnrollmentRequest) -> Result<(), EnrollmentError> {
    validate_key_request(&request.public_key, &request.key_algorithm)?;
    for (label, value) in [
        ("enrollment_token", request.enrollment_token.as_str()),
        ("machine_id", request.machine_id.as_str()),
        (
            "hardware_fingerprint",
            request.hardware_fingerprint.as_str(),
        ),
        ("agent_version", request.agent_version.as_str()),
        ("benchmark_version", request.benchmark_version.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(EnrollmentError::Invalid(format!("{label} is required")));
        }
    }
    if contains_secret_field(&request.registration_payload) {
        return Err(EnrollmentError::Invalid(
            "registration payload contains a forbidden secret field".to_string(),
        ));
    }
    if request.registration_payload["secrets_included"] == serde_json::Value::Bool(true) {
        return Err(EnrollmentError::Invalid(
            "registration payload declares secret material".to_string(),
        ));
    }
    validate_claim_binding(
        &request.registration_payload,
        "machine_id",
        &request.machine_id,
    )?;
    validate_claim_binding(
        &request.registration_payload,
        "hardware_fingerprint",
        &request.hardware_fingerprint,
    )?;
    validate_claim_binding(
        &request.registration_payload,
        "public_key",
        &request.public_key,
    )?;
    if let Some(local_provider_id) = request.local_provider_id.as_deref() {
        validate_claim_binding(
            &request.registration_payload,
            "provider_id",
            local_provider_id,
        )?;
    }
    Ok(())
}

fn validate_claim_binding(
    payload: &serde_json::Value,
    field: &str,
    expected: &str,
) -> Result<(), EnrollmentError> {
    if let Some(actual) = payload.get(field).and_then(serde_json::Value::as_str)
        && actual != expected
    {
        return Err(EnrollmentError::Invalid(format!(
            "registration payload {field} does not match enrollment request"
        )));
    }
    Ok(())
}

fn contains_secret_field(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "private_key"
                    | "secret_key"
                    | "secret_key_base64"
                    | "api_token"
                    | "enrollment_token"
                    | "credential"
                    | "authorization"
            ) || contains_secret_field(value)
        }),
        serde_json::Value::Array(items) => items.iter().any(contains_secret_field),
        _ => false,
    }
}

fn validate_key_request(public_key: &str, algorithm: &str) -> Result<(), EnrollmentError> {
    if algorithm != KEY_ALGORITHM {
        return Err(EnrollmentError::Invalid(format!(
            "unsupported key algorithm '{algorithm}'"
        )));
    }
    validate_public_key(public_key).map_err(EnrollmentError::Invalid)
}

fn is_expired(value: &str) -> Result<bool, EnrollmentError> {
    let timestamp = DateTime::parse_from_rfc3339(value)
        .map_err(|error| EnrollmentError::Database(DbError::new(error.to_string())))?
        .with_timezone(&Utc);
    Ok(timestamp <= Utc::now())
}

fn device_from_row(row: Row) -> DeviceRecord {
    DeviceRecord {
        device_id: row.get("device_id"),
        provider_id: row.get("provider_id"),
        machine_id: row.get("machine_id"),
        status: row.get("status"),
        active_public_key_id: row.get("active_public_key_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CreateProviderCommand, CreateProviderOutcome};
    use burd_protocol::{
        EnrollmentProofRequest, KeyRotationProofRequest, StartEnrollmentRequest,
        StartKeyRotationRequest, enrollment_proof_message, generate_keypair,
        key_rotation_proof_message, sign_message,
    };

    #[test]
    fn enrollment_request_rejects_secret_fields_and_mismatched_claims() {
        let keys = generate_keypair().unwrap();
        let mut request = StartEnrollmentRequest {
            enrollment_token: "burd_enroll_test".to_string(),
            public_key: keys.public_key_base64,
            key_algorithm: KEY_ALGORITHM.to_string(),
            local_provider_id: Some("local-provider".to_string()),
            machine_id: "machine-1".to_string(),
            registration_payload: serde_json::json!({
                "provider_id": "local-provider",
                "machine_id": "machine-1",
                "hardware_fingerprint": "sha256:one",
                "secrets_included": false
            }),
            hardware_fingerprint: "sha256:one".to_string(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: "0.1.0".to_string(),
        };
        assert!(validate_enrollment_request(&request).is_ok());

        request.registration_payload["nested"] =
            serde_json::json!({"private_key": "must-not-persist"});
        assert!(validate_enrollment_request(&request).is_err());

        request.registration_payload["nested"] = serde_json::json!({});
        request.registration_payload["machine_id"] = serde_json::json!("other-machine");
        assert!(validate_enrollment_request(&request).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn remote_enrollment_rotation_and_revocation_flow() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_enrollment_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let created = db
            .create_provider_idempotently(CreateProviderCommand {
                request_id: "req_provider".to_string(),
                scope: "POST /v1/providers".to_string(),
                idempotency_key: "provider-enrollment-test".to_string(),
                request_hash: "provider-request-hash".to_string(),
                user_id: None,
                display_name: Some("Enrollment Provider".to_string()),
            })
            .await
            .unwrap();
        let CreateProviderOutcome::Response(created) = created else {
            panic!("provider must be created");
        };
        let created: serde_json::Value = serde_json::from_str(&created.response_json).unwrap();
        let provider_id = created["provider"]["provider_id"].as_str().unwrap();

        let issued = db
            .issue_enrollment_token(provider_id, "req_token", 600)
            .await
            .unwrap();
        let keys = generate_keypair().unwrap();
        let start_request = StartEnrollmentRequest {
            enrollment_token: issued.enrollment_token,
            public_key: keys.public_key_base64.clone(),
            key_algorithm: KEY_ALGORITHM.to_string(),
            local_provider_id: Some("local-provider".to_string()),
            machine_id: "machine-integration".to_string(),
            registration_payload: serde_json::json!({
                "provider_id": "local-provider",
                "machine_id": "machine-integration",
                "secrets_included": false
            }),
            hardware_fingerprint: "sha256:integration".to_string(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: "0.1.0".to_string(),
        };
        let started = db
            .start_enrollment("req_start", &start_request, 300)
            .await
            .unwrap();
        let replayed = db
            .start_enrollment("req_start_replay", &start_request, 300)
            .await
            .unwrap();
        assert_eq!(replayed.enrollment_id, started.enrollment_id);
        assert_eq!(replayed.nonce, started.nonce);

        let message = enrollment_proof_message(
            &started.enrollment_id,
            &started.provider_id,
            &start_request.machine_id,
            &started.nonce,
            &keys.public_key_base64,
            &start_request.hardware_fingerprint,
            &started.expires_at,
        )
        .unwrap();
        let signature = sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap();
        let enrolled = db
            .complete_enrollment(
                &started.enrollment_id,
                "req_proof",
                &EnrollmentProofRequest {
                    nonce: started.nonce,
                    signature,
                    public_key: keys.public_key_base64,
                    hardware_fingerprint: start_request.hardware_fingerprint,
                },
                900,
            )
            .await
            .unwrap();
        assert_eq!(enrolled.status, "pending_verification");
        assert!(matches!(
            db.complete_enrollment(
                &started.enrollment_id,
                "req_replay",
                &EnrollmentProofRequest {
                    nonce: "used".to_string(),
                    signature: "used".to_string(),
                    public_key: "used".to_string(),
                    hardware_fingerprint: "used".to_string(),
                },
                900,
            )
            .await,
            Err(EnrollmentError::NonceReused)
        ));

        let refreshed = db
            .refresh_device_credential(
                &enrolled.device_id,
                &enrolled.credential,
                "req_refresh",
                900,
            )
            .await
            .unwrap();
        assert!(matches!(
            db.refresh_device_credential(
                &enrolled.device_id,
                &enrolled.credential,
                "req_old_credential",
                900,
            )
            .await,
            Err(EnrollmentError::Revoked)
        ));

        let next_keys = generate_keypair().unwrap();
        let rotation = db
            .start_key_rotation(
                &enrolled.device_id,
                &refreshed.credential,
                "req_rotation",
                &StartKeyRotationRequest {
                    new_public_key: next_keys.public_key_base64.clone(),
                    key_algorithm: KEY_ALGORITHM.to_string(),
                },
                300,
            )
            .await
            .unwrap();
        let rotation_message = key_rotation_proof_message(
            &rotation.rotation_id,
            &rotation.provider_id,
            &rotation.device_id,
            &rotation.nonce,
            &rotation.current_public_key_id,
            &next_keys.public_key_base64,
            &rotation.expires_at,
        )
        .unwrap();
        let rotation_signature =
            sign_message(&next_keys.secret_key_base64, rotation_message.as_bytes()).unwrap();
        let rotated = db
            .complete_key_rotation(
                &rotation.device_id,
                &rotation.rotation_id,
                &refreshed.credential,
                "req_rotation_proof",
                &KeyRotationProofRequest {
                    nonce: rotation.nonce,
                    signature: rotation_signature,
                    new_public_key: next_keys.public_key_base64,
                },
            )
            .await
            .unwrap();
        assert_ne!(rotated.public_key_id, enrolled.public_key_id);

        let devices = db.list_provider_devices(provider_id).await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices[0].active_public_key_id.as_deref(),
            Some(rotated.public_key_id.as_str())
        );

        let revoked = db
            .revoke_device(&enrolled.device_id, "req_revoke")
            .await
            .unwrap();
        assert_eq!(revoked.status, "revoked");
        assert!(matches!(
            db.refresh_device_credential(
                &enrolled.device_id,
                &refreshed.credential,
                "req_revoked_credential",
                900,
            )
            .await,
            Err(EnrollmentError::Revoked)
        ));

        db.drop_schema_for_test().await.unwrap();
    }
}
