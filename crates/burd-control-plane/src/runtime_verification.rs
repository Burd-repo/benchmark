use crate::Database;
use crate::db::{NewAuditEvent, insert_audit_event};
use crate::remote_session::{AuthorizedSession, SessionError};
use burd_protocol::{
    AGENT_RUNTIME_CONTRACT_VERSION, IssueRuntimeVerificationChallengeRequest,
    IssueRuntimeVerificationChallengeResponse, ListProviderRuntimeVerificationsResponse,
    NextRuntimeVerificationChallengeResponse, ProviderRuntimeVerificationRecord,
    RUNTIME_PROOF_POLICY_VERSION, RUNTIME_VERIFICATION_CHALLENGE_SCHEMA_VERSION,
    RUNTIME_VERIFICATION_RECORD_SCHEMA_VERSION, RuntimeVerificationChallenge,
    RuntimeVerificationChallengeRecord, SignedRuntimeVerificationResponse,
    SubmitRuntimeVerificationResponse, fingerprint_claims, immutable_image_ref, random_token,
    runtime_admission_claims_from_verification, runtime_admission_fingerprint,
    runtime_verification_fingerprint, runtime_verification_response_hash,
    runtime_verification_signature_message, validate_provider_runtime_verification_record,
    validate_runtime_verification_challenge, validate_runtime_verification_evidence,
    validate_signed_runtime_verification_response, verify_message,
};
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::Row;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RuntimeVerificationPolicy {
    pub challenge_ttl_seconds: u32,
    pub clock_skew_seconds: u32,
    pub verification_ttl_seconds: u32,
    pub approved_proof_image_ref: Option<String>,
}

impl Database {
    pub async fn issue_runtime_verification_challenge(
        &self,
        request_id: &str,
        request: &IssueRuntimeVerificationChallengeRequest,
        policy: RuntimeVerificationPolicy,
    ) -> Result<IssueRuntimeVerificationChallengeResponse, SessionError> {
        let (challenge_ttl, verification_ttl) = validate_issue_request(request, policy)?;
        let host_os = host_os_for_backend(&request.runtime_backend)?;
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
                "runtime verification requires an online or degraded session".to_string(),
            ));
        }
        let hardware_fingerprint: Option<String> = session.get("hardware_fingerprint");
        let hardware_fingerprint = hardware_fingerprint.ok_or_else(|| {
            SessionError::Conflict(
                "runtime verification requires a session hardware fingerprint".to_string(),
            )
        })?;
        let gpu_exists = transaction
            .query_opt(
                "SELECT 1 FROM device_gpu_inventory WHERE provider_id = $1 AND device_id = $2 AND lower(gpu_uuid) = lower($3) AND status = 'active' AND snapshot_id = (SELECT snapshot_id FROM device_gpu_inventory_snapshots WHERE provider_id = $1 AND device_id = $2 ORDER BY ingest_seq DESC LIMIT 1) LIMIT 1",
                &[&request.provider_id, &request.device_id, &request.gpu_uuid],
            )
            .await?
            .is_some();
        if !gpu_exists {
            return Err(SessionError::Conflict(
                "runtime verification GPU is absent from the device inventory".to_string(),
            ));
        }
        let active = transaction
            .query_opt(
                "SELECT challenge_id FROM runtime_verification_challenges WHERE provider_id = $1 AND device_id = $2 AND gpu_uuid = $3 AND status IN ('issued', 'acknowledged') AND expires_at > $4 LIMIT 1",
                &[&request.provider_id, &request.device_id, &request.gpu_uuid, &Utc::now().to_rfc3339()],
            )
            .await?;
        if active.is_some() {
            return Err(SessionError::Conflict(
                "an active runtime verification challenge already exists for this GPU".to_string(),
            ));
        }

        let now = Utc::now();
        let issued_at = now.to_rfc3339();
        let expires_at = (now + Duration::seconds(i64::from(challenge_ttl))).to_rfc3339();
        let challenge_id = format!("runtime_challenge_{}", Uuid::new_v4());
        let nonce = random_token("burd_runtime").map_err(SessionError::Invalid)?;
        let challenge = RuntimeVerificationChallenge {
            schema_version: RUNTIME_VERIFICATION_CHALLENGE_SCHEMA_VERSION.to_string(),
            challenge_id: challenge_id.clone(),
            nonce,
            provider_id: request.provider_id.clone(),
            device_id: request.device_id.clone(),
            session_id: request.session_id.clone(),
            hardware_fingerprint: hardware_fingerprint.clone(),
            host_os: host_os.to_string(),
            gpu_uuid: request.gpu_uuid.clone(),
            runtime_backend: request.runtime_backend.clone(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            proof_image_ref: request.proof_image_ref.clone(),
            proof_policy_version: RUNTIME_PROOF_POLICY_VERSION.to_string(),
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            issued_at: issued_at.clone(),
            expires_at: expires_at.clone(),
            verification_ttl_seconds: verification_ttl,
        };
        validate_runtime_verification_challenge(&challenge).map_err(SessionError::Invalid)?;
        let challenge_json = serde_json::to_string(&challenge)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO runtime_verification_challenges (challenge_id, provider_id, device_id, session_id, gpu_uuid, runtime_backend, hardware_fingerprint, status, nonce, challenge_json, verification_ttl_seconds, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7, 'issued', $8, $9, $10, $11, $12)",
                &[
                    &challenge_id,
                    &request.provider_id,
                    &request.device_id,
                    &request.session_id,
                    &request.gpu_uuid,
                    &request.runtime_backend,
                    &hardware_fingerprint,
                    &challenge.nonce,
                    &challenge_json,
                    &(verification_ttl as i32),
                    &issued_at,
                    &expires_at,
                ],
            )
            .await?;
        let metadata = serde_json::json!({
            "provider_id": request.provider_id,
            "device_id": request.device_id,
            "session_id": request.session_id,
            "gpu_uuid": request.gpu_uuid,
            "runtime_backend": request.runtime_backend,
            "proof_image_ref": request.proof_image_ref,
            "expires_at": expires_at,
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "runtime_verification_challenge",
                entity_id: &challenge_id,
                event_type: "runtime_verification_challenge.issued",
                idempotency_key: None,
                summary: "authoritative runtime verification challenge issued",
                metadata_json: &metadata,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(IssueRuntimeVerificationChallengeResponse {
            request_id: request_id.to_string(),
            challenge,
        })
    }

    pub async fn next_runtime_verification_challenge(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
    ) -> Result<NextRuntimeVerificationChallengeResponse, SessionError> {
        let now = Utc::now().to_rfc3339();
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .execute(
                "UPDATE runtime_verification_challenges SET status = 'expired', expired_at = $2 WHERE session_id = $1 AND status IN ('issued', 'acknowledged') AND expires_at <= $2",
                &[&authorized.session_id, &now],
            )
            .await?;
        let row = transaction
            .query_opt(
                "SELECT challenge_id, challenge_json FROM runtime_verification_challenges WHERE session_id = $1 AND provider_id = $2 AND device_id = $3 AND status IN ('issued', 'acknowledged') AND expires_at > $4 ORDER BY issued_at LIMIT 1 FOR UPDATE SKIP LOCKED",
                &[&authorized.session_id, &authorized.provider_id, &authorized.device_id, &now],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("runtime verification challenge not found".to_string()))?;
        let challenge_id: String = row.get("challenge_id");
        let challenge = challenge_from_json(row.get("challenge_json"))?;
        transaction
            .execute(
                "UPDATE runtime_verification_challenges SET status = 'acknowledged', acknowledged_at = COALESCE(acknowledged_at, $2) WHERE challenge_id = $1",
                &[&challenge_id, &now],
            )
            .await?;
        transaction.commit().await?;
        Ok(NextRuntimeVerificationChallengeResponse {
            request_id: request_id.to_string(),
            challenge,
        })
    }

    pub async fn get_runtime_verification_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<Option<RuntimeVerificationChallengeRecord>, SessionError> {
        let client = self.connect().await?;
        client
            .query_opt(
                "SELECT challenge_json, status, response_hash, public_key_id, acknowledged_at, submitted_at, verified_at, failed_at, expired_at, verification_json FROM runtime_verification_challenges WHERE challenge_id = $1",
                &[&challenge_id],
            )
            .await?
            .map(challenge_record_from_row)
            .transpose()
    }

    pub async fn list_provider_runtime_verifications(
        &self,
        request_id: &str,
        provider_id: &str,
    ) -> Result<ListProviderRuntimeVerificationsResponse, SessionError> {
        let current_time = Utc::now();
        let now = current_time.to_rfc3339();
        let client = self.connect().await?;
        client
            .execute(
                "UPDATE provider_runtime_verifications SET status = 'expired' WHERE provider_id = $1 AND status = 'verified' AND expires_at <= $2",
                &[&provider_id, &now],
            )
            .await?;
        let rows = client
            .query(
                "SELECT record_json FROM provider_runtime_verifications WHERE provider_id = $1 AND status = 'verified' AND expires_at > $2 ORDER BY verified_at DESC",
                &[&provider_id, &now],
            )
            .await?;
        let verifications = rows
            .into_iter()
            .map(|row| {
                let record = verification_from_json(row.get("record_json"))?;
                validate_provider_runtime_verification_record(&record, current_time)
                    .map_err(SessionError::Invalid)?;
                Ok::<ProviderRuntimeVerificationRecord, SessionError>(record)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListProviderRuntimeVerificationsResponse {
            request_id: request_id.to_string(),
            verifications,
        })
    }

    pub async fn submit_runtime_verification_response(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        session_id: &str,
        challenge_id: &str,
        signed: &SignedRuntimeVerificationResponse,
        policy: RuntimeVerificationPolicy,
    ) -> Result<SubmitRuntimeVerificationResponse, SessionError> {
        if session_id != authorized.session_id
            || signed.payload.session_id != authorized.session_id
            || signed.payload.provider_id != authorized.provider_id
            || signed.payload.device_id != authorized.device_id
            || signed.payload.challenge_id != challenge_id
        {
            return Err(SessionError::Unauthorized);
        }
        validate_signed_runtime_verification_response(signed).map_err(SessionError::Invalid)?;
        let computed_hash =
            runtime_verification_response_hash(&signed.payload).map_err(SessionError::Invalid)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_opt(
                "SELECT challenge_json, status, verification_ttl_seconds, expires_at FROM runtime_verification_challenges WHERE challenge_id = $1 FOR UPDATE",
                &[&challenge_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("runtime verification challenge not found".to_string()))?;
        let status: String = row.get("status");
        if !matches!(status.as_str(), "issued" | "acknowledged") {
            return Err(SessionError::Conflict(format!(
                "runtime verification challenge is not accepting responses in status {status}"
            )));
        }
        let challenge = challenge_from_json(row.get("challenge_json"))?;
        if challenge.provider_id != authorized.provider_id
            || challenge.device_id != authorized.device_id
            || challenge.session_id != authorized.session_id
        {
            return Err(SessionError::Unauthorized);
        }
        let now = Utc::now();
        let server_received_at = now.to_rfc3339();
        if parse_time(row.get("expires_at"))? <= now {
            transaction
                .execute(
                    "UPDATE runtime_verification_challenges SET status = 'expired', expired_at = $2 WHERE challenge_id = $1",
                    &[&challenge_id, &server_received_at],
                )
                .await?;
            transaction.commit().await?;
            return Err(SessionError::Expired);
        }
        let replay = transaction
            .query_opt(
                "SELECT challenge_id FROM runtime_verification_challenges WHERE response_hash = $1 AND challenge_id <> $2 LIMIT 1",
                &[&computed_hash, &challenge_id],
            )
            .await?
            .is_some();
        if replay {
            return Err(SessionError::Conflict(
                "runtime verification response hash was already submitted".to_string(),
            ));
        }
        let public_key: Option<String> = transaction
            .query_opt(
                "SELECT public_key FROM provider_public_keys WHERE public_key_id = $1 AND provider_id = $2 AND device_id = $3 AND status = 'active'",
                &[&signed.public_key_id, &authorized.provider_id, &authorized.device_id],
            )
            .await?
            .map(|row| row.get("public_key"));
        let reason_codes = response_reason_codes(
            &challenge,
            signed,
            &computed_hash,
            public_key.as_deref(),
            now,
            policy.clock_skew_seconds,
        );
        let accepted = reason_codes.is_empty();
        let mut verification = None;
        if accepted {
            let verification_id = format!("runtime_verification_{}", Uuid::new_v4());
            let ttl: i32 = row.get("verification_ttl_seconds");
            let expires_at = (now + Duration::seconds(i64::from(ttl))).to_rfc3339();
            let admission_claims =
                runtime_admission_claims_from_verification(&challenge, &signed.payload.evidence);
            let admission_fingerprint =
                runtime_admission_fingerprint(&admission_claims).map_err(SessionError::Invalid)?;
            let record = ProviderRuntimeVerificationRecord {
                schema_version: RUNTIME_VERIFICATION_RECORD_SCHEMA_VERSION.to_string(),
                verification_id: verification_id.clone(),
                challenge_id: challenge_id.to_string(),
                provider_id: authorized.provider_id.clone(),
                device_id: authorized.device_id.clone(),
                session_id: authorized.session_id.clone(),
                hardware_fingerprint: challenge.hardware_fingerprint.clone(),
                gpu_uuid: challenge.gpu_uuid.clone(),
                host_os: challenge.host_os.clone(),
                runtime_backend: challenge.runtime_backend.clone(),
                status: "verified".to_string(),
                gpu_uuid_binding: "verified".to_string(),
                runtime_verification_fingerprint: signed
                    .payload
                    .runtime_verification_fingerprint
                    .clone(),
                proof_policy_version: challenge.proof_policy_version.clone(),
                agent_runtime_contract_version: challenge.agent_runtime_contract_version.clone(),
                proof_image_digest: challenge.proof_image_ref.clone(),
                public_key_id: Some(signed.public_key_id.clone()),
                runtime_admission_fingerprint: Some(admission_fingerprint.clone()),
                runtime_admission_claims: Some(admission_claims.clone()),
                verified_at: server_received_at.clone(),
                expires_at: expires_at.clone(),
                reason_codes: Vec::new(),
            };
            validate_provider_runtime_verification_record(&record, now)
                .map_err(SessionError::Invalid)?;
            let record_json = serde_json::to_string(&record)
                .map_err(|error| SessionError::Invalid(error.to_string()))?;
            let admission_claims_json = serde_json::to_string(&admission_claims)
                .map_err(|error| SessionError::Invalid(error.to_string()))?;
            transaction
                .execute(
                    "UPDATE provider_runtime_verifications SET status = 'superseded' WHERE provider_id = $1 AND device_id = $2 AND gpu_uuid = $3 AND status = 'verified'",
                    &[&authorized.provider_id, &authorized.device_id, &challenge.gpu_uuid],
                )
                .await?;
            transaction
                .execute(
                    "INSERT INTO provider_runtime_verifications (verification_id, challenge_id, provider_id, device_id, session_id, gpu_uuid, runtime_backend, hardware_fingerprint, runtime_verification_fingerprint, status, verified_at, expires_at, record_json, public_key_id, runtime_admission_fingerprint, runtime_admission_claims_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'verified', $10, $11, $12, $13, $14, $15)",
                    &[
                        &verification_id,
                        &challenge_id,
                        &authorized.provider_id,
                        &authorized.device_id,
                        &authorized.session_id,
                        &challenge.gpu_uuid,
                        &challenge.runtime_backend,
                        &challenge.hardware_fingerprint,
                        &record.runtime_verification_fingerprint,
                        &server_received_at,
                        &expires_at,
                        &record_json,
                        &signed.public_key_id,
                        &admission_fingerprint,
                        &admission_claims_json,
                    ],
                )
                .await?;
            verification = Some(record);
        }
        let next_status = if accepted { "verified" } else { "failed" };
        let response_json = serde_json::to_string(signed)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let verification_json = verification
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        transaction
            .execute(
                "UPDATE runtime_verification_challenges SET status = $1, submitted_at = $2, verified_at = CASE WHEN $1 = 'verified' THEN $2 ELSE verified_at END, failed_at = CASE WHEN $1 = 'failed' THEN $2 ELSE failed_at END, response_hash = $3, public_key_id = $4, response_json = $5, verification_json = $6 WHERE challenge_id = $7",
                &[
                    &next_status,
                    &server_received_at,
                    &computed_hash,
                    &signed.public_key_id,
                    &response_json,
                    &verification_json,
                    &challenge_id,
                ],
            )
            .await?;
        let metadata = serde_json::json!({
            "status": next_status,
            "response_hash": computed_hash,
            "runtime_verification_fingerprint": signed.payload.runtime_verification_fingerprint,
            "reason_codes": reason_codes,
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_key",
                actor_id: Some(signed.public_key_id.clone()),
                entity_type: "runtime_verification_challenge",
                entity_id: challenge_id,
                event_type: if accepted {
                    "runtime_verification.verified"
                } else {
                    "runtime_verification.failed"
                },
                idempotency_key: None,
                summary: if accepted {
                    "provider runtime capability verified"
                } else {
                    "provider runtime capability proof rejected"
                },
                metadata_json: &metadata,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(SubmitRuntimeVerificationResponse {
            request_id: request_id.to_string(),
            challenge_id: challenge_id.to_string(),
            status: next_status.to_string(),
            response_hash: computed_hash,
            server_received_at,
            verification,
            reason_codes,
        })
    }
}

fn validate_issue_request(
    request: &IssueRuntimeVerificationChallengeRequest,
    policy: RuntimeVerificationPolicy,
) -> Result<(u32, u32), SessionError> {
    let challenge_ttl = request.ttl_seconds.unwrap_or(policy.challenge_ttl_seconds);
    let verification_ttl = request
        .verification_ttl_seconds
        .unwrap_or(policy.verification_ttl_seconds);
    let approved_proof_image_ref = policy.approved_proof_image_ref.as_deref().ok_or_else(|| {
        SessionError::Conflict("runtime proof policy is not configured".to_string())
    })?;
    if !safe_id(&request.provider_id)
        || !safe_id(&request.device_id)
        || !safe_id(&request.session_id)
        || request.gpu_uuid.is_empty()
        || request.gpu_uuid.len() > 128
        || !request.gpu_uuid.is_ascii()
        || request.gpu_uuid.chars().any(char::is_whitespace)
        || !immutable_image_ref(&request.proof_image_ref)
        || request.proof_image_ref != approved_proof_image_ref
        || challenge_ttl == 0
        || challenge_ttl > policy.challenge_ttl_seconds
        || verification_ttl == 0
        || verification_ttl > policy.verification_ttl_seconds
        || verification_ttl > 604_800
    {
        return Err(SessionError::Invalid(
            "runtime verification challenge request is invalid".to_string(),
        ));
    }
    host_os_for_backend(&request.runtime_backend)?;
    Ok((challenge_ttl, verification_ttl))
}

fn response_reason_codes(
    challenge: &RuntimeVerificationChallenge,
    signed: &SignedRuntimeVerificationResponse,
    computed_hash: &str,
    public_key: Option<&str>,
    now: DateTime<Utc>,
    clock_skew_seconds: u32,
) -> Vec<String> {
    let mut reasons = Vec::new();
    let payload = &signed.payload;
    if signed.response_hash != computed_hash {
        reasons.push("response_hash_mismatch".to_string());
    }
    if payload.challenge_id != challenge.challenge_id
        || payload.nonce != challenge.nonce
        || payload.provider_id != challenge.provider_id
        || payload.device_id != challenge.device_id
        || payload.session_id != challenge.session_id
        || payload.hardware_fingerprint != challenge.hardware_fingerprint
        || payload.gpu_uuid != challenge.gpu_uuid
        || payload.runtime_backend != challenge.runtime_backend
        || payload.proof_policy_version != challenge.proof_policy_version
        || payload.agent_runtime_contract_version != challenge.agent_runtime_contract_version
    {
        reasons.push("challenge_binding_mismatch".to_string());
    }
    if validate_runtime_verification_evidence(challenge, &payload.evidence).is_err() {
        reasons.push("evidence_invalid".to_string());
    }
    match runtime_verification_fingerprint(&fingerprint_claims(challenge, &payload.evidence)) {
        Ok(expected) if expected == payload.runtime_verification_fingerprint => {}
        _ => reasons.push("fingerprint_mismatch".to_string()),
    }
    let skew = Duration::seconds(i64::from(clock_skew_seconds));
    let times_valid = parse_time(&challenge.issued_at)
        .and_then(|issued| {
            let expires = parse_time(&challenge.expires_at)?;
            let started = parse_time(&payload.started_at)?;
            let completed = parse_time(&payload.completed_at)?;
            Ok(started >= issued - skew
                && completed >= started
                && completed < expires
                && completed <= now + skew)
        })
        .unwrap_or(false);
    if !times_valid {
        reasons.push("response_time_invalid".to_string());
    }
    match public_key {
        Some(public_key) => {
            let valid = runtime_verification_signature_message(
                payload,
                computed_hash,
                &signed.public_key_id,
            )
            .and_then(|message| verify_message(public_key, message.as_bytes(), &signed.signature))
            .unwrap_or(false);
            if !valid {
                reasons.push("signature_invalid".to_string());
            }
        }
        None => reasons.push("public_key_unavailable".to_string()),
    }
    reasons
}

fn challenge_record_from_row(row: Row) -> Result<RuntimeVerificationChallengeRecord, SessionError> {
    let verification_json: Option<String> = row.get("verification_json");
    Ok(RuntimeVerificationChallengeRecord {
        challenge: challenge_from_json(row.get("challenge_json"))?,
        status: row.get("status"),
        response_hash: row.get("response_hash"),
        public_key_id: row.get("public_key_id"),
        acknowledged_at: row.get("acknowledged_at"),
        submitted_at: row.get("submitted_at"),
        verified_at: row.get("verified_at"),
        failed_at: row.get("failed_at"),
        expired_at: row.get("expired_at"),
        verification: verification_json
            .map(|value| verification_from_json(&value))
            .transpose()?,
    })
}

fn challenge_from_json(value: &str) -> Result<RuntimeVerificationChallenge, SessionError> {
    let challenge: RuntimeVerificationChallenge =
        serde_json::from_str(value).map_err(|error| SessionError::Invalid(error.to_string()))?;
    validate_runtime_verification_challenge(&challenge).map_err(SessionError::Invalid)?;
    Ok(challenge)
}

fn verification_from_json(value: &str) -> Result<ProviderRuntimeVerificationRecord, SessionError> {
    serde_json::from_str(value).map_err(|error| SessionError::Invalid(error.to_string()))
}

fn host_os_for_backend(runtime_backend: &str) -> Result<&'static str, SessionError> {
    match runtime_backend {
        "docker_linux_native" => Ok("linux"),
        "docker_wsl2" => Ok("windows"),
        _ => Err(SessionError::Invalid(
            "runtime verification backend is unsupported".to_string(),
        )),
    }
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, SessionError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| SessionError::Invalid("runtime verification timestamp is invalid".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{
        RUNTIME_VERIFICATION_CANONICALIZATION_VERSION,
        RUNTIME_VERIFICATION_RESPONSE_SCHEMA_VERSION, RuntimeVerificationEvidence,
        RuntimeVerificationResponsePayload, generate_keypair, runtime_verification_response_hash,
        runtime_verification_signature_message, sign_message,
    };

    fn policy() -> RuntimeVerificationPolicy {
        RuntimeVerificationPolicy {
            challenge_ttl_seconds: 600,
            clock_skew_seconds: 300,
            verification_ttl_seconds: 86_400,
            approved_proof_image_ref: Some(format!("ghcr.io/burd/proof@sha256:{}", "a".repeat(64))),
        }
    }

    #[test]
    fn issue_policy_rejects_unpinned_images_and_excessive_ttl() {
        let mut request = IssueRuntimeVerificationChallengeRequest {
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            runtime_backend: "docker_linux_native".to_string(),
            proof_image_ref: format!("ghcr.io/burd/proof@sha256:{}", "a".repeat(64)),
            ttl_seconds: Some(600),
            verification_ttl_seconds: Some(86_400),
        };
        assert_eq!(
            validate_issue_request(&request, policy()).unwrap(),
            (600, 86_400)
        );
        request.proof_image_ref = "ghcr.io/burd/proof:latest".to_string();
        assert!(validate_issue_request(&request, policy()).is_err());
        request.proof_image_ref = format!("ghcr.io/burd/proof@sha256:{}", "a".repeat(64));
        request.ttl_seconds = Some(601);
        assert!(validate_issue_request(&request, policy()).is_err());
        request.ttl_seconds = Some(600);
        request.proof_image_ref = format!("ghcr.io/burd/proof@sha256:{}", "b".repeat(64));
        assert!(validate_issue_request(&request, policy()).is_err());
    }

    fn challenge(now: DateTime<Utc>) -> RuntimeVerificationChallenge {
        RuntimeVerificationChallenge {
            schema_version: RUNTIME_VERIFICATION_CHALLENGE_SCHEMA_VERSION.to_string(),
            challenge_id: "runtime_challenge_1".to_string(),
            nonce: "burd_runtime_nonce_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            hardware_fingerprint: "a".repeat(64),
            host_os: "linux".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            runtime_backend: "docker_linux_native".to_string(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            proof_image_ref: format!("ghcr.io/burd/proof@sha256:{}", "b".repeat(64)),
            proof_policy_version: RUNTIME_PROOF_POLICY_VERSION.to_string(),
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            issued_at: (now - Duration::seconds(5)).to_rfc3339(),
            expires_at: (now + Duration::minutes(5)).to_rfc3339(),
            verification_ttl_seconds: 86_400,
        }
    }

    fn evidence(challenge: &RuntimeVerificationChallenge) -> RuntimeVerificationEvidence {
        RuntimeVerificationEvidence {
            host_os: challenge.host_os.clone(),
            runtime_backend: challenge.runtime_backend.clone(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            docker_server_version: "27.1.1".to_string(),
            nvidia_driver_version: "560.35".to_string(),
            nvidia_runtime: "nvidia".to_string(),
            cuda_runtime_version: "12.6".to_string(),
            observed_gpu_uuids: vec![challenge.gpu_uuid.clone()],
            proof_image_digest: challenge.proof_image_ref.clone(),
            proof_nonce: challenge.nonce.clone(),
            network_mode: "none".to_string(),
            run_as_user: "1000:1000".to_string(),
            read_only_rootfs: true,
            no_new_privileges: true,
            cap_drop: vec!["ALL".to_string()],
        }
    }

    fn signed_response(
        challenge: &RuntimeVerificationChallenge,
        now: DateTime<Utc>,
    ) -> (SignedRuntimeVerificationResponse, String) {
        let evidence = evidence(challenge);
        let fingerprint =
            runtime_verification_fingerprint(&fingerprint_claims(challenge, &evidence)).unwrap();
        let payload = RuntimeVerificationResponsePayload {
            schema_version: RUNTIME_VERIFICATION_RESPONSE_SCHEMA_VERSION.to_string(),
            challenge_id: challenge.challenge_id.clone(),
            nonce: challenge.nonce.clone(),
            provider_id: challenge.provider_id.clone(),
            device_id: challenge.device_id.clone(),
            session_id: challenge.session_id.clone(),
            hardware_fingerprint: challenge.hardware_fingerprint.clone(),
            gpu_uuid: challenge.gpu_uuid.clone(),
            runtime_backend: challenge.runtime_backend.clone(),
            proof_policy_version: challenge.proof_policy_version.clone(),
            agent_runtime_contract_version: challenge.agent_runtime_contract_version.clone(),
            runtime_verification_fingerprint: fingerprint,
            evidence,
            started_at: (now - Duration::seconds(2)).to_rfc3339(),
            completed_at: (now - Duration::seconds(1)).to_rfc3339(),
        };
        let response_hash = runtime_verification_response_hash(&payload).unwrap();
        let key = generate_keypair().unwrap();
        let message =
            runtime_verification_signature_message(&payload, &response_hash, "key_1").unwrap();
        let signature = sign_message(&key.secret_key_base64, message.as_bytes()).unwrap();
        (
            SignedRuntimeVerificationResponse {
                payload,
                response_hash,
                public_key_id: "key_1".to_string(),
                signature,
                canonicalization_version: RUNTIME_VERIFICATION_CANONICALIZATION_VERSION.to_string(),
            },
            key.public_key_base64,
        )
    }

    #[test]
    fn authoritative_verifier_rejects_tamper_and_expired_execution() {
        let now = Utc::now();
        let challenge = challenge(now);
        let (signed, public_key) = signed_response(&challenge, now);
        let computed = runtime_verification_response_hash(&signed.payload).unwrap();
        assert!(
            response_reason_codes(&challenge, &signed, &computed, Some(&public_key), now, 300)
                .is_empty()
        );

        let mut tampered = signed.clone();
        tampered
            .payload
            .evidence
            .observed_gpu_uuids
            .push("GPU-other".to_string());
        let tampered_hash = runtime_verification_response_hash(&tampered.payload).unwrap();
        let reasons = response_reason_codes(
            &challenge,
            &tampered,
            &tampered_hash,
            Some(&public_key),
            now,
            300,
        );
        assert!(reasons.iter().any(|reason| reason == "evidence_invalid"));
        assert!(
            reasons
                .iter()
                .any(|reason| reason == "response_hash_mismatch")
        );
        assert!(reasons.iter().any(|reason| reason == "signature_invalid"));

        let mut expired = signed;
        expired.payload.completed_at = challenge.expires_at.clone();
        let expired_hash = runtime_verification_response_hash(&expired.payload).unwrap();
        let reasons = response_reason_codes(
            &challenge,
            &expired,
            &expired_hash,
            Some(&public_key),
            now,
            300,
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason == "response_time_invalid")
        );
    }
}
