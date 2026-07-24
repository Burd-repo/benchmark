use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::{AuthorizedSession, SessionError};
use crate::verification_policy::{
    VerificationChallengeContext, VerificationPolicy, record_challenge_issued_in_transaction,
};
use burd_protocol::{
    IssueProofChallengeRequest, IssueProofChallengeResponse, NextProofChallengeResponse,
    PROOF_CHALLENGE_CANONICALIZATION_VERSION, PROOF_CHALLENGE_RESPONSE_SCHEMA_VERSION,
    PROOF_CHALLENGE_SCHEMA_VERSION, ProofCapabilityChallenge, ProofChallengeRecord,
    ProofChallengeVerification, SignedProofCapabilityResponse, SubmitProofChallengeResponse,
    proof_capability_response_hash, proof_capability_response_signature_message, random_token,
    verify_message,
};
use chrono::{DateTime, Duration, Utc};
use std::fs;
use std::path::{Path, PathBuf};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const DEFAULT_REQUIRED_PROOFS: &[&str] = &[
    "cuda_runtime",
    "vram_allocation_residency",
    "tensor_gemm_microbenchmark",
    "llm_short_inference",
    "performance_consistency",
    "contention_detection",
    "telemetry_window",
];

#[derive(Debug, Clone, Copy)]
pub struct ProofChallengePolicy {
    pub ttl_seconds: u32,
    pub clock_skew_seconds: u32,
}

impl Database {
    pub async fn issue_proof_challenge(
        &self,
        request_id: &str,
        request: &IssueProofChallengeRequest,
        policy: ProofChallengePolicy,
    ) -> Result<IssueProofChallengeResponse, SessionError> {
        self.issue_proof_challenge_with_context(request_id, request, policy, None)
            .await
    }

    pub(crate) async fn issue_proof_challenge_with_context(
        &self,
        request_id: &str,
        request: &IssueProofChallengeRequest,
        policy: ProofChallengePolicy,
        verification_context: Option<VerificationChallengeContext>,
    ) -> Result<IssueProofChallengeResponse, SessionError> {
        let required_proofs = normalized_required_proofs(request)?;
        let ttl_seconds = validate_issue_request(request, &required_proofs, policy)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let session = transaction
            .query_opt(
                "SELECT s.status, s.hardware_fingerprint, p.status AS provider_status, d.status AS device_status FROM provider_sessions s JOIN providers p ON p.provider_id = s.provider_id JOIN devices d ON d.device_id = s.device_id WHERE s.session_id = $1 AND s.provider_id = $2 AND s.device_id = $3 AND d.provider_id = $2 FOR UPDATE",
                &[&request.session_id, &request.provider_id, &request.device_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
        let provider_status: String = session.get("provider_status");
        let device_status: String = session.get("device_status");
        if matches!(provider_status.as_str(), "blocked" | "quarantined")
            || device_status != "active"
        {
            return Err(SessionError::Revoked);
        }
        let session_status: String = session.get("status");
        if !matches!(session_status.as_str(), "online" | "degraded") {
            return Err(SessionError::Conflict(
                "proof challenge requires an online or degraded remote session".to_string(),
            ));
        }
        let session_fingerprint: Option<String> = session.get("hardware_fingerprint");
        if session_fingerprint.as_deref() != Some(request.required_fingerprint.as_str()) {
            return Err(SessionError::Conflict(
                "proof challenge required_fingerprint does not match the remote session"
                    .to_string(),
            ));
        }

        let now = Utc::now();
        let issued_at = now.to_rfc3339();
        let expires_at = (now + Duration::seconds(i64::from(ttl_seconds))).to_rfc3339();
        let challenge_id = format!("proof_challenge_{}", Uuid::new_v4());
        let nonce = random_token("burd_poc").map_err(SessionError::Invalid)?;
        let required_proofs_json = serde_json::to_string(&required_proofs)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO proof_challenges (challenge_id, provider_id, device_id, session_id, status, nonce, schema_version, profile_version, required_fingerprint, required_gpu_uuid, required_backend, model_artifact_hash, prompt_seed, required_proofs_json, min_tokens_per_second, max_ttft_ms, issued_at, expires_at) VALUES ($1, $2, $3, $4, 'issued', $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
                &[
                    &challenge_id,
                    &request.provider_id,
                    &request.device_id,
                    &request.session_id,
                    &nonce,
                    &PROOF_CHALLENGE_SCHEMA_VERSION,
                    &request.profile_version,
                    &request.required_fingerprint,
                    &request.required_gpu_uuid,
                    &request.required_backend,
                    &request.model_artifact_hash,
                    &request.prompt_seed,
                    &required_proofs_json,
                    &request.min_tokens_per_second,
                    &(request.max_ttft_ms as i64),
                    &issued_at,
                    &expires_at,
                ],
            )
            .await?;
        let challenge = ProofCapabilityChallenge {
            schema_version: PROOF_CHALLENGE_SCHEMA_VERSION.to_string(),
            challenge_id: challenge_id.clone(),
            nonce: nonce.clone(),
            provider_id: request.provider_id.clone(),
            device_id: request.device_id.clone(),
            session_id: request.session_id.clone(),
            profile_version: request.profile_version.clone(),
            required_fingerprint: request.required_fingerprint.clone(),
            required_gpu_uuid: request.required_gpu_uuid.clone(),
            required_backend: request.required_backend.clone(),
            model_artifact_hash: request.model_artifact_hash.clone(),
            prompt_seed: request.prompt_seed.clone(),
            required_proofs,
            min_tokens_per_second: request.min_tokens_per_second,
            max_ttft_ms: request.max_ttft_ms,
            issued_at: issued_at.clone(),
            expires_at: expires_at.clone(),
        };
        if let Some(context) = verification_context.as_ref() {
            record_challenge_issued_in_transaction(&transaction, request_id, &challenge, context)
                .await?;
        }
        let audit_metadata = serde_json::json!({
            "provider_id": request.provider_id,
            "device_id": request.device_id,
            "session_id": request.session_id,
            "profile_version": request.profile_version,
            "required_gpu_uuid": request.required_gpu_uuid,
            "required_backend": request.required_backend,
            "expires_at": expires_at,
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "proof_challenge",
                entity_id: &challenge_id,
                event_type: "proof_challenge.issued",
                idempotency_key: None,
                summary: "active proof-of-capability challenge issued",
                metadata_json: &audit_metadata,
            },
        )
        .await?;
        transaction.commit().await?;

        Ok(IssueProofChallengeResponse {
            request_id: request_id.to_string(),
            challenge,
        })
    }
    pub async fn next_proof_challenge(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
    ) -> Result<NextProofChallengeResponse, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        expire_stale_proof_challenges(&transaction, &now).await?;
        let row = transaction
            .query_opt(
                &format!(
                    "{} WHERE session_id = $1 AND provider_id = $2 AND device_id = $3 AND status IN ('issued', 'acknowledged') ORDER BY issued_at ASC LIMIT 1 FOR UPDATE",
                    proof_challenge_select_columns()
                ),
                &[&authorized.session_id, &authorized.provider_id, &authorized.device_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("proof challenge not found".to_string()))?;
        let challenge_id: String = row.get("challenge_id");
        let status: String = row.get("status");
        if status == "issued" {
            transaction
                .execute(
                    "UPDATE proof_challenges SET status = 'acknowledged', acknowledged_at = COALESCE(acknowledged_at, $1) WHERE challenge_id = $2",
                    &[&now, &challenge_id],
                )
                .await?;
            let audit_metadata = serde_json::json!({
                "session_id": authorized.session_id,
                "device_id": authorized.device_id,
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "device",
                    actor_id: Some(authorized.device_id.clone()),
                    entity_type: "proof_challenge",
                    entity_id: &challenge_id,
                    event_type: "proof_challenge.acknowledged",
                    idempotency_key: None,
                    summary: "proof-of-capability challenge delivered to session",
                    metadata_json: &audit_metadata,
                },
            )
            .await?;
        }
        let record = proof_challenge_record_from_row(row)?;
        transaction.commit().await?;
        Ok(NextProofChallengeResponse {
            request_id: request_id.to_string(),
            challenge: record.challenge,
        })
    }

    pub async fn get_proof_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<Option<ProofChallengeRecord>, SessionError> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                &format!(
                    "{} WHERE challenge_id = $1",
                    proof_challenge_select_columns()
                ),
                &[&challenge_id],
            )
            .await?;
        row.map(proof_challenge_record_from_row).transpose()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn submit_proof_challenge_response(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        session_id: &str,
        challenge_id: &str,
        signed: &SignedProofCapabilityResponse,
        object_storage_dir: &str,
        policy: ProofChallengePolicy,
        verification_policy: VerificationPolicy,
    ) -> Result<SubmitProofChallengeResponse, SessionError> {
        if session_id != authorized.session_id
            || signed.payload.session_id != authorized.session_id
            || signed.payload.device_id != authorized.device_id
            || signed.payload.provider_id != authorized.provider_id
            || signed.payload.challenge_id != challenge_id
        {
            return Err(SessionError::Unauthorized);
        }
        validate_signed_response_shape(signed)?;
        let computed_response_hash =
            proof_capability_response_hash(&signed.payload).map_err(SessionError::Invalid)?;

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_opt(
                &format!(
                    "{} WHERE challenge_id = $1 FOR UPDATE",
                    proof_challenge_select_columns()
                ),
                &[&challenge_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("proof challenge not found".to_string()))?;
        let status: String = row.get("status");
        let challenge = proof_challenge_record_from_row(row)?.challenge;
        if challenge.provider_id != authorized.provider_id
            || challenge.device_id != authorized.device_id
            || challenge.session_id != authorized.session_id
        {
            return Err(SessionError::Unauthorized);
        }
        if !matches!(status.as_str(), "issued" | "acknowledged" | "running") {
            return Err(SessionError::Conflict(format!(
                "proof challenge is not accepting responses in status {status}"
            )));
        }

        let now = Utc::now();
        let server_received_at = now.to_rfc3339();
        if timestamp_expired_at(&challenge.expires_at, now)? {
            mark_proof_challenge_expired(
                &transaction,
                request_id,
                authorized,
                challenge_id,
                &server_received_at,
            )
            .await?;
            transaction.commit().await?;
            return Err(SessionError::Expired);
        }

        let key = transaction
            .query_opt(
                "SELECT public_key FROM provider_public_keys WHERE public_key_id = $1 AND provider_id = $2 AND device_id = $3 AND status = 'active'",
                &[&signed.public_key_id, &authorized.provider_id, &authorized.device_id],
            )
            .await?;
        let public_key: Option<String> = key.map(|row| row.get("public_key"));
        let mut verification = build_proof_verification(
            &challenge,
            signed,
            public_key.as_deref(),
            &computed_response_hash,
            now,
            policy,
        );
        if !validate_telemetry_window_link(
            &transaction,
            &challenge,
            signed,
            &mut verification.errors,
        )
        .await?
        {
            verification.metrics_satisfied = false;
        }
        let accepted = verification.errors.is_empty();
        let next_status = if accepted { "verified" } else { "failed" };
        let response_object_key = write_proof_response_object(
            object_storage_dir,
            &authorized.provider_id,
            challenge_id,
            &computed_response_hash,
            signed,
        )
        .await?;
        let response_json = serde_json::to_string(signed)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let verification_json = serde_json::to_string(&verification)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        transaction
            .execute(
                "UPDATE proof_challenges SET status = $1, started_at = $2, submitted_at = $3, verified_at = CASE WHEN $1 = 'verified' THEN $3 ELSE verified_at END, failed_at = CASE WHEN $1 = 'failed' THEN $3 ELSE failed_at END, response_hash = $4, public_key_id = $5, response_object_key = $6, response_json = $7, verification_json = $8 WHERE challenge_id = $9",
                &[
                    &next_status,
                    &Some(signed.payload.started_at.clone()),
                    &server_received_at,
                    &Some(computed_response_hash.clone()),
                    &Some(signed.public_key_id.clone()),
                    &Some(response_object_key.clone()),
                    &Some(response_json),
                    &Some(verification_json),
                    &challenge_id,
                ],
            )
            .await?;
        self.record_proof_challenge_outcome(
            &transaction,
            request_id,
            &challenge,
            &verification,
            accepted,
            verification_policy,
            &server_received_at,
        )
        .await?;
        let audit_metadata = serde_json::json!({
            "status": next_status,
            "response_hash": computed_response_hash,
            "submitted_response_hash": signed.response_hash,
            "response_object_key": response_object_key,
            "errors": verification.errors,
            "warnings": verification.warnings,
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_key",
                actor_id: Some(signed.public_key_id.clone()),
                entity_type: "proof_challenge",
                entity_id: challenge_id,
                event_type: if accepted {
                    "proof_challenge.verified"
                } else {
                    "proof_challenge.failed"
                },
                idempotency_key: None,
                summary: if accepted {
                    "proof-of-capability response verified"
                } else {
                    "proof-of-capability response failed backend verification"
                },
                metadata_json: &audit_metadata,
            },
        )
        .await?;
        transaction.commit().await?;

        Ok(SubmitProofChallengeResponse {
            request_id: request_id.to_string(),
            challenge_id: challenge_id.to_string(),
            status: next_status.to_string(),
            response_hash: computed_response_hash,
            server_received_at,
            verification,
        })
    }
}

fn normalized_required_proofs(
    request: &IssueProofChallengeRequest,
) -> Result<Vec<String>, SessionError> {
    let proofs: Vec<String> = if request.required_proofs.is_empty() {
        DEFAULT_REQUIRED_PROOFS
            .iter()
            .map(|proof| (*proof).to_string())
            .collect()
    } else {
        request.required_proofs.clone()
    };
    if proofs.len() > 32
        || proofs
            .iter()
            .any(|proof| !is_safe_identifier(proof, 64) || proof.contains("secret"))
    {
        return Err(SessionError::Invalid(
            "required_proofs must contain short ASCII identifiers".to_string(),
        ));
    }
    Ok(proofs)
}

fn validate_issue_request(
    request: &IssueProofChallengeRequest,
    required_proofs: &[String],
    policy: ProofChallengePolicy,
) -> Result<u32, SessionError> {
    let ttl_seconds = request.expires_in_seconds.unwrap_or(policy.ttl_seconds);
    if ttl_seconds == 0 || ttl_seconds > policy.ttl_seconds {
        return Err(SessionError::Invalid(format!(
            "proof challenge TTL must be between 1 and {} seconds",
            policy.ttl_seconds
        )));
    }
    if !request.min_tokens_per_second.is_finite() || request.min_tokens_per_second < 0.0 {
        return Err(SessionError::Invalid(
            "min_tokens_per_second must be finite and nonnegative".to_string(),
        ));
    }
    if request.max_ttft_ms > i64::MAX as u64 {
        return Err(SessionError::Invalid(
            "max_ttft_ms exceeds the supported range".to_string(),
        ));
    }
    for (label, value, max_len) in [
        ("provider_id", request.provider_id.as_str(), 96),
        ("device_id", request.device_id.as_str(), 96),
        ("session_id", request.session_id.as_str(), 96),
        ("profile_version", request.profile_version.as_str(), 96),
        (
            "required_fingerprint",
            request.required_fingerprint.as_str(),
            160,
        ),
        ("required_backend", request.required_backend.as_str(), 32),
        (
            "model_artifact_hash",
            request.model_artifact_hash.as_str(),
            160,
        ),
        ("prompt_seed", request.prompt_seed.as_str(), 160),
    ] {
        if !is_bounded_ascii(value, max_len) {
            return Err(SessionError::Invalid(format!(
                "{label} is empty or contains unsupported characters"
            )));
        }
    }
    if request
        .required_gpu_uuid
        .as_deref()
        .is_some_and(|value| !is_bounded_ascii(value, 128))
    {
        return Err(SessionError::Invalid(
            "required_gpu_uuid contains unsupported characters".to_string(),
        ));
    }
    if required_proofs.is_empty() {
        return Err(SessionError::Invalid(
            "at least one proof profile is required".to_string(),
        ));
    }
    Ok(ttl_seconds)
}

fn validate_signed_response_shape(
    signed: &SignedProofCapabilityResponse,
) -> Result<(), SessionError> {
    if signed.canonicalization_version != PROOF_CHALLENGE_CANONICALIZATION_VERSION
        || signed.payload.schema_version != PROOF_CHALLENGE_RESPONSE_SCHEMA_VERSION
    {
        return Err(SessionError::Invalid(
            "unsupported proof response schema or canonicalization version".to_string(),
        ));
    }
    for (label, value, max_len) in [
        ("challenge_id", signed.payload.challenge_id.as_str(), 96),
        ("nonce", signed.payload.nonce.as_str(), 128),
        ("provider_id", signed.payload.provider_id.as_str(), 96),
        ("device_id", signed.payload.device_id.as_str(), 96),
        ("session_id", signed.payload.session_id.as_str(), 96),
        (
            "profile_version",
            signed.payload.profile_version.as_str(),
            96,
        ),
        (
            "hardware_fingerprint",
            signed.payload.hardware_fingerprint.as_str(),
            160,
        ),
        ("gpu_uuid", signed.payload.gpu_uuid.as_str(), 128),
        ("backend", signed.payload.backend.as_str(), 32),
        (
            "model_artifact_hash",
            signed.payload.model_artifact_hash.as_str(),
            160,
        ),
        ("prompt_seed", signed.payload.prompt_seed.as_str(), 160),
        ("driver_version", signed.payload.driver_version.as_str(), 64),
        ("response_hash", signed.response_hash.as_str(), 128),
        ("public_key_id", signed.public_key_id.as_str(), 96),
    ] {
        if !is_bounded_ascii(value, max_len) {
            return Err(SessionError::Invalid(format!(
                "{label} is empty or contains unsupported characters"
            )));
        }
    }
    if signed.payload.metrics.backend_proof.trim().is_empty()
        || signed.payload.metrics.backend_proof.len() > 128
    {
        return Err(SessionError::Invalid(
            "backend_proof is empty or too long".to_string(),
        ));
    }
    for value in [
        signed.payload.metrics.tokens_per_second,
        signed.payload.metrics.gemm_gflops,
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(SessionError::Invalid(
                "proof floating-point metrics must be finite and nonnegative".to_string(),
            ));
        }
    }
    Ok(())
}

fn build_proof_verification(
    challenge: &ProofCapabilityChallenge,
    signed: &SignedProofCapabilityResponse,
    active_public_key: Option<&str>,
    computed_response_hash: &str,
    checked_at: DateTime<Utc>,
    policy: ProofChallengePolicy,
) -> ProofChallengeVerification {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let payload = &signed.payload;

    let response_hash_valid = constant_time_eq(
        computed_response_hash.as_bytes(),
        signed.response_hash.as_bytes(),
    );
    if !response_hash_valid {
        errors.push("proof response hash does not match the canonical payload".to_string());
    }

    let signature_valid = active_public_key.is_some_and(|public_key| {
        proof_capability_response_signature_message(
            payload,
            &signed.response_hash,
            &signed.public_key_id,
        )
        .ok()
        .and_then(|message| verify_message(public_key, message.as_bytes(), &signed.signature).ok())
        .unwrap_or(false)
    });
    if active_public_key.is_none() {
        errors.push("proof response public_key_id is not the active backend key".to_string());
    } else if !signature_valid {
        errors.push("proof response Ed25519 signature invalid for active backend key".to_string());
    }

    let provider_bound = payload.provider_id == challenge.provider_id;
    let device_bound = payload.device_id == challenge.device_id;
    let session_bound = payload.session_id == challenge.session_id;
    let fingerprint_bound = payload.hardware_fingerprint == challenge.required_fingerprint;
    let gpu_bound = challenge
        .required_gpu_uuid
        .as_deref()
        .map(|required| payload.gpu_uuid == required)
        .unwrap_or_else(|| !payload.gpu_uuid.trim().is_empty());
    let backend_bound = payload.backend == challenge.required_backend;
    let artifact_bound = payload.model_artifact_hash == challenge.model_artifact_hash;
    let prompt_bound = payload.prompt_seed == challenge.prompt_seed;

    push_if_false(
        &mut errors,
        provider_bound,
        "proof response provider_id mismatch",
    );
    push_if_false(
        &mut errors,
        device_bound,
        "proof response device_id mismatch",
    );
    push_if_false(
        &mut errors,
        session_bound,
        "proof response session_id mismatch",
    );
    push_if_false(
        &mut errors,
        fingerprint_bound,
        "proof response hardware fingerprint mismatch",
    );
    push_if_false(&mut errors, gpu_bound, "proof response GPU UUID mismatch");
    push_if_false(
        &mut errors,
        backend_bound,
        "proof response backend mismatch",
    );
    push_if_false(
        &mut errors,
        artifact_bound,
        "proof response model artifact hash mismatch",
    );
    push_if_false(
        &mut errors,
        prompt_bound,
        "proof response prompt seed mismatch",
    );

    let expired_by_server = timestamp_expired_at(&challenge.expires_at, checked_at).unwrap_or(true);
    if expired_by_server {
        errors.push("proof challenge expired by server clock".to_string());
    }
    validate_response_timestamps(challenge, payload, checked_at, policy, &mut errors);
    let metrics_satisfied = validate_metrics(challenge, payload, &mut warnings, &mut errors);

    ProofChallengeVerification {
        schema_version: PROOF_CHALLENGE_SCHEMA_VERSION.to_string(),
        challenge_id: challenge.challenge_id.clone(),
        checked_at: checked_at.to_rfc3339(),
        response_hash_valid,
        signature_valid,
        provider_bound,
        device_bound,
        session_bound,
        fingerprint_bound,
        gpu_bound,
        backend_bound,
        artifact_bound,
        prompt_bound,
        metrics_satisfied,
        expired_by_server,
        warnings,
        errors,
    }
}

async fn validate_telemetry_window_link(
    transaction: &Transaction<'_>,
    challenge: &ProofCapabilityChallenge,
    signed: &SignedProofCapabilityResponse,
    errors: &mut Vec<String>,
) -> Result<bool, SessionError> {
    if !challenge
        .required_proofs
        .iter()
        .any(|proof| proof == "telemetry_window")
    {
        return Ok(true);
    }
    let payload = &signed.payload;
    let Some(window_hash) = payload
        .telemetry_window_hash
        .as_deref()
        .filter(|value| is_bounded_ascii(value, 160))
    else {
        return Ok(false);
    };
    let row = transaction
        .query_opt(
            "SELECT tb.batch_id, EXISTS(SELECT 1 FROM gpu_telemetry_samples sample WHERE sample.batch_id = tb.batch_id AND sample.gpu_uuid = $6) AS gpu_observed FROM telemetry_batches tb WHERE tb.batch_hash = $1 AND tb.provider_id = $2 AND tb.device_id = $3 AND tb.session_id = $4 AND tb.hardware_fingerprint = $5",
            &[
                &window_hash,
                &challenge.provider_id,
                &challenge.device_id,
                &challenge.session_id,
                &challenge.required_fingerprint,
                &payload.gpu_uuid,
            ],
        )
        .await?;
    let Some(row) = row else {
        errors.push(
            "proof response telemetry window hash is not a verified telemetry batch for this session"
                .to_string(),
        );
        return Ok(false);
    };
    if !row.get::<_, bool>("gpu_observed") {
        errors.push(
            "proof response telemetry window does not include the proof GPU UUID".to_string(),
        );
        return Ok(false);
    }
    Ok(true)
}
fn validate_response_timestamps(
    challenge: &ProofCapabilityChallenge,
    payload: &burd_protocol::ProofCapabilityResponsePayload,
    checked_at: DateTime<Utc>,
    policy: ProofChallengePolicy,
    errors: &mut Vec<String>,
) {
    let Ok(issued_at) = parse_timestamp(&challenge.issued_at) else {
        errors.push("proof challenge issued_at is invalid".to_string());
        return;
    };
    let Ok(expires_at) = parse_timestamp(&challenge.expires_at) else {
        errors.push("proof challenge expires_at is invalid".to_string());
        return;
    };
    let Ok(started_at) = parse_timestamp(&payload.started_at) else {
        errors.push("proof response started_at is invalid".to_string());
        return;
    };
    let Ok(completed_at) = parse_timestamp(&payload.completed_at) else {
        errors.push("proof response completed_at is invalid".to_string());
        return;
    };
    if started_at > completed_at {
        errors.push("proof response execution window is reversed".to_string());
    }
    let skew = Duration::seconds(i64::from(policy.clock_skew_seconds));
    if started_at < issued_at - skew {
        errors.push("proof response started before challenge issuance".to_string());
    }
    if completed_at > expires_at + skew {
        errors.push("proof response completed after challenge expiration".to_string());
    }
    if completed_at > checked_at + skew {
        errors.push("proof response completed_at is ahead of the server clock".to_string());
    }
}

fn validate_metrics(
    challenge: &ProofCapabilityChallenge,
    payload: &burd_protocol::ProofCapabilityResponsePayload,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> bool {
    let baseline_errors = errors.len();
    let metrics = &payload.metrics;
    if challenge.min_tokens_per_second > 0.0 {
        match metrics.tokens_per_second {
            Some(value) if value.is_finite() && value >= challenge.min_tokens_per_second => {}
            _ => errors
                .push("proof response tokens_per_second is below challenge minimum".to_string()),
        }
    }
    if challenge.max_ttft_ms > 0 {
        match metrics.ttft_ms {
            Some(value) if value <= challenge.max_ttft_ms => {}
            _ => errors.push("proof response ttft_ms exceeds challenge maximum".to_string()),
        }
    }
    if challenge.required_backend == "cuda" {
        if !metrics.cuda_runtime_detected {
            errors.push("proof response did not prove CUDA runtime availability".to_string());
        }
        if payload
            .cuda_runtime_version
            .as_deref()
            .is_none_or(str::is_empty)
        {
            errors.push("proof response missing CUDA runtime version".to_string());
        }
    }
    if challenge
        .required_proofs
        .iter()
        .any(|proof| proof == "vram_allocation_residency")
    {
        match (metrics.vram_allocated_mib, metrics.vram_resident_mib) {
            (Some(allocated), Some(resident))
                if allocated > 0 && resident > 0 && resident <= allocated => {}
            _ => errors.push("proof response did not prove VRAM allocation residency".to_string()),
        }
    }
    if challenge
        .required_proofs
        .iter()
        .any(|proof| proof == "tensor_gemm_microbenchmark")
        && !metrics
            .gemm_gflops
            .is_some_and(|value| value.is_finite() && value > 0.0)
    {
        errors.push("proof response missing Tensor/GEMM microbenchmark metric".to_string());
    }
    if challenge
        .required_proofs
        .iter()
        .any(|proof| proof == "llm_short_inference")
        && (metrics.tokens_per_second.is_none() || metrics.ttft_ms.is_none())
    {
        errors.push("proof response missing short LLM inference metrics".to_string());
    }
    if challenge
        .required_proofs
        .iter()
        .any(|proof| proof == "contention_detection")
        && metrics.contention_detected
    {
        errors.push("proof response detected GPU contention".to_string());
    }
    if challenge
        .required_proofs
        .iter()
        .any(|proof| proof == "telemetry_window")
        && payload
            .telemetry_window_hash
            .as_deref()
            .is_none_or(|value| !is_bounded_ascii(value, 160))
    {
        errors.push("proof response missing telemetry window hash".to_string());
    }
    if challenge
        .required_proofs
        .iter()
        .any(|proof| proof == "performance_consistency")
    {
        warnings.push(
            "performance consistency uses challenge history once recurring verification is enabled"
                .to_string(),
        );
    }
    errors.len() == baseline_errors
}

async fn expire_stale_proof_challenges(
    transaction: &Transaction<'_>,
    now: &str,
) -> Result<(), SessionError> {
    transaction
        .execute(
            "UPDATE proof_challenges SET status = 'expired', expired_at = COALESCE(expired_at, $1) WHERE status IN ('issued', 'acknowledged', 'running') AND expires_at <= $1",
            &[&now],
        )
        .await?;
    Ok(())
}

async fn mark_proof_challenge_expired(
    transaction: &Transaction<'_>,
    request_id: &str,
    authorized: &AuthorizedSession,
    challenge_id: &str,
    now: &str,
) -> Result<(), SessionError> {
    transaction
        .execute(
            "UPDATE proof_challenges SET status = 'expired', expired_at = COALESCE(expired_at, $1) WHERE challenge_id = $2",
            &[&now, &challenge_id],
        )
        .await?;
    let metadata = serde_json::json!({
        "session_id": authorized.session_id,
        "device_id": authorized.device_id,
        "expired_at": now,
    })
    .to_string();
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id,
            actor_type: "device",
            actor_id: Some(authorized.device_id.clone()),
            entity_type: "proof_challenge",
            entity_id: challenge_id,
            event_type: "proof_challenge.expired",
            idempotency_key: None,
            summary: "proof-of-capability response arrived after server expiration",
            metadata_json: &metadata,
        },
    )
    .await?;
    Ok(())
}

fn proof_challenge_record_from_row(row: Row) -> Result<ProofChallengeRecord, SessionError> {
    let required_proofs_json: String = row.get("required_proofs_json");
    let required_proofs: Vec<String> = serde_json::from_str(&required_proofs_json)
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    let verification = row
        .get::<_, Option<String>>("verification_json")
        .map(|raw| serde_json::from_str(&raw))
        .transpose()
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    let issued_at: String = row.get("issued_at");
    Ok(ProofChallengeRecord {
        challenge: ProofCapabilityChallenge {
            schema_version: row.get("schema_version"),
            challenge_id: row.get("challenge_id"),
            nonce: row.get("nonce"),
            provider_id: row.get("provider_id"),
            device_id: row.get("device_id"),
            session_id: row.get("session_id"),
            profile_version: row.get("profile_version"),
            required_fingerprint: row.get("required_fingerprint"),
            required_gpu_uuid: row.get("required_gpu_uuid"),
            required_backend: row.get("required_backend"),
            model_artifact_hash: row.get("model_artifact_hash"),
            prompt_seed: row.get("prompt_seed"),
            required_proofs,
            min_tokens_per_second: row.get("min_tokens_per_second"),
            max_ttft_ms: row.get::<_, i64>("max_ttft_ms").max(0) as u64,
            issued_at: issued_at.clone(),
            expires_at: row.get("expires_at"),
        },
        status: row.get("status"),
        response_hash: row.get("response_hash"),
        public_key_id: row.get("public_key_id"),
        response_object_key: row.get("response_object_key"),
        issued_at,
        acknowledged_at: row.get("acknowledged_at"),
        started_at: row.get("started_at"),
        submitted_at: row.get("submitted_at"),
        verified_at: row.get("verified_at"),
        failed_at: row.get("failed_at"),
        expired_at: row.get("expired_at"),
        verification,
    })
}

fn proof_challenge_select_columns() -> &'static str {
    "SELECT challenge_id, provider_id, device_id, session_id, status, nonce, schema_version, profile_version, required_fingerprint, required_gpu_uuid, required_backend, model_artifact_hash, prompt_seed, required_proofs_json, min_tokens_per_second, max_ttft_ms, issued_at, expires_at, acknowledged_at, started_at, submitted_at, verified_at, failed_at, expired_at, response_hash, public_key_id, response_object_key, verification_json FROM proof_challenges"
}

async fn write_proof_response_object(
    root: &str,
    provider_id: &str,
    challenge_id: &str,
    response_hash: &str,
    signed: &SignedProofCapabilityResponse,
) -> Result<String, SessionError> {
    if !is_safe_identifier(provider_id, 96)
        || !is_safe_identifier(challenge_id, 96)
        || !is_hex_hash(response_hash)
    {
        return Err(SessionError::Invalid(
            "proof response object key contains unsupported characters".to_string(),
        ));
    }
    let object_key = format!("proof-challenges/{provider_id}/{challenge_id}/{response_hash}.json");
    let path = object_path(root, &object_key)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create proof response object directory", parent, error))?;
    }
    let bytes = serde_json::to_vec_pretty(signed)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    fs::write(&path, bytes)
        .map_err(|error| io_error("write proof response object", &path, error))?;
    Ok(object_key)
}

fn object_path(root: &str, object_key: &str) -> Result<PathBuf, SessionError> {
    let mut path = PathBuf::from(root);
    for component in object_key.split('/') {
        if !is_safe_identifier(component.trim_end_matches(".json"), 128) {
            return Err(SessionError::Invalid(
                "proof response object key contains unsupported characters".to_string(),
            ));
        }
        path.push(component);
    }
    Ok(path)
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> SessionError {
    SessionError::Database(DbError::new(format!(
        "failed to {action} {}: {error}",
        path.display()
    )))
}

fn parse_timestamp(raw: &str) -> Result<DateTime<Utc>, SessionError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| SessionError::Invalid(format!("invalid proof timestamp: {error}")))
}

fn timestamp_expired_at(raw: &str, now: DateTime<Utc>) -> Result<bool, SessionError> {
    Ok(parse_timestamp(raw)? <= now)
}

fn push_if_false(errors: &mut Vec<String>, condition: bool, message: &str) {
    if !condition {
        errors.push(message.to_string());
    }
}

fn is_bounded_ascii(value: &str, max_len: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= max_len
        && trimmed
            .chars()
            .all(|character| character.is_ascii() && !character.is_ascii_control())
}

fn is_safe_identifier(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn is_hex_hash(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{
        PROOF_CHALLENGE_RESPONSE_SCHEMA_VERSION, ProofCapabilityMetrics,
        ProofCapabilityResponsePayload, sign_message,
    };

    fn challenge() -> ProofCapabilityChallenge {
        ProofCapabilityChallenge {
            schema_version: PROOF_CHALLENGE_SCHEMA_VERSION.to_string(),
            challenge_id: "proof_challenge_1".to_string(),
            nonce: "nonce_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            profile_version: "poc-cuda-llm-v1".to_string(),
            required_fingerprint: "sha256:fingerprint".to_string(),
            required_gpu_uuid: Some("GPU-test".to_string()),
            required_backend: "cuda".to_string(),
            model_artifact_hash: "sha256:model".to_string(),
            prompt_seed: "seed_1".to_string(),
            required_proofs: DEFAULT_REQUIRED_PROOFS
                .iter()
                .map(|proof| (*proof).to_string())
                .collect(),
            min_tokens_per_second: 10.0,
            max_ttft_ms: 500,
            issued_at: "2026-07-09T00:00:00Z".to_string(),
            expires_at: "2026-07-09T00:10:00Z".to_string(),
        }
    }

    fn payload() -> ProofCapabilityResponsePayload {
        ProofCapabilityResponsePayload {
            schema_version: PROOF_CHALLENGE_RESPONSE_SCHEMA_VERSION.to_string(),
            challenge_id: "proof_challenge_1".to_string(),
            nonce: "nonce_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            profile_version: "poc-cuda-llm-v1".to_string(),
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            backend: "cuda".to_string(),
            model_artifact_hash: "sha256:model".to_string(),
            prompt_seed: "seed_1".to_string(),
            driver_version: "576.80".to_string(),
            cuda_driver_version: Some("12.9".to_string()),
            cuda_runtime_version: Some("12.8".to_string()),
            metrics: ProofCapabilityMetrics {
                tokens_per_second: Some(42.0),
                ttft_ms: Some(120),
                vram_allocated_mib: Some(4096),
                vram_resident_mib: Some(4096),
                gemm_gflops: Some(9000.0),
                cuda_runtime_detected: true,
                backend_proof: "cuda-device-query".to_string(),
                contention_detected: false,
            },
            telemetry_window_hash: Some("sha256:telemetry".to_string()),
            started_at: "2026-07-09T00:00:01Z".to_string(),
            completed_at: "2026-07-09T00:00:05Z".to_string(),
        }
    }

    #[test]
    fn verification_accepts_signed_matching_response() {
        let keys = burd_protocol::generate_keypair().unwrap();
        let payload = payload();
        let response_hash = proof_capability_response_hash(&payload).unwrap();
        let message =
            proof_capability_response_signature_message(&payload, &response_hash, "key_1").unwrap();
        let signed = SignedProofCapabilityResponse {
            payload,
            response_hash: response_hash.clone(),
            public_key_id: "key_1".to_string(),
            signature: sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap(),
            canonicalization_version: PROOF_CHALLENGE_CANONICALIZATION_VERSION.to_string(),
        };

        let verification = build_proof_verification(
            &challenge(),
            &signed,
            Some(&keys.public_key_base64),
            &response_hash,
            DateTime::parse_from_rfc3339("2026-07-09T00:00:06Z")
                .unwrap()
                .with_timezone(&Utc),
            ProofChallengePolicy {
                ttl_seconds: 600,
                clock_skew_seconds: 300,
            },
        );

        assert!(verification.errors.is_empty(), "{:?}", verification.errors);
        assert!(verification.signature_valid);
        assert!(verification.metrics_satisfied);
    }

    #[test]
    fn verification_rejects_below_threshold_metrics() {
        let keys = burd_protocol::generate_keypair().unwrap();
        let mut payload = payload();
        payload.metrics.tokens_per_second = Some(3.0);
        let response_hash = proof_capability_response_hash(&payload).unwrap();
        let message =
            proof_capability_response_signature_message(&payload, &response_hash, "key_1").unwrap();
        let signed = SignedProofCapabilityResponse {
            payload,
            response_hash: response_hash.clone(),
            public_key_id: "key_1".to_string(),
            signature: sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap(),
            canonicalization_version: PROOF_CHALLENGE_CANONICALIZATION_VERSION.to_string(),
        };

        let verification = build_proof_verification(
            &challenge(),
            &signed,
            Some(&keys.public_key_base64),
            &response_hash,
            DateTime::parse_from_rfc3339("2026-07-09T00:00:06Z")
                .unwrap()
                .with_timezone(&Utc),
            ProofChallengePolicy {
                ttl_seconds: 600,
                clock_skew_seconds: 300,
            },
        );

        assert!(!verification.metrics_satisfied);
        assert!(
            verification
                .errors
                .iter()
                .any(|error| error.contains("tokens_per_second"))
        );
    }
}
