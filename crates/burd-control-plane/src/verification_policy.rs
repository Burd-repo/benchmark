use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::proof_challenge::ProofChallengePolicy;
use crate::remote_session::SessionError;
use burd_protocol::{
    IssueProofChallengeRequest, ListVerificationStatesResponse, ProofCapabilityChallenge,
    ProofChallengeVerification, RunVerificationSweepRequest, RunVerificationSweepResponse,
    VERIFICATION_POLICY_VERSION, VerificationStateRecord, VerificationSweepIssuedChallenge,
    random_token,
};
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::{Row, Transaction};

const DEFAULT_PROFILE_VERSION: &str = "poc-cuda-llm-v1";
const DEFAULT_REQUIRED_BACKEND: &str = "cuda";
const DEFAULT_MODEL_ARTIFACT_HASH: &str = "sha256:burd-poc-v1";

#[derive(Debug, Clone, Copy)]
pub struct VerificationPolicy {
    pub period_seconds: u32,
    pub retry_budget: u32,
    pub sweep_limit: u32,
    pub suspect_failures: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct VerificationChallengeContext {
    pub reason: String,
    pub retry_budget: u32,
}

#[derive(Debug, Clone)]
struct VerificationCandidate {
    provider_id: String,
    device_id: String,
    session_id: String,
    hardware_fingerprint: String,
    latest_gpu_uuid: Option<String>,
    state_status: Option<String>,
    next_due_at: Option<String>,
    has_active_challenge: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FailureTransition {
    pub status: &'static str,
    pub retry_budget_remaining: i32,
    pub next_due_at: Option<String>,
}

impl Database {
    pub async fn run_verification_sweep(
        &self,
        request_id: &str,
        request: &RunVerificationSweepRequest,
        proof_policy: ProofChallengePolicy,
        verification_policy: VerificationPolicy,
    ) -> Result<RunVerificationSweepResponse, SessionError> {
        validate_sweep_request(request)?;
        self.expire_stale_verification_challenges(request_id, verification_policy)
            .await?;

        let now = Utc::now();
        let limit = request
            .limit
            .unwrap_or(verification_policy.sweep_limit)
            .min(verification_policy.sweep_limit);
        let reason = sweep_reason(request);
        let candidates = self.verification_candidates(limit).await?;
        let evaluated = candidates.len() as u32;
        let mut issued = Vec::new();

        for candidate in candidates {
            if !should_issue_challenge(&candidate, now, request.force) {
                continue;
            }
            let prompt_seed = random_token("burd_poc_seed").map_err(SessionError::Invalid)?;
            let response = match self
                .issue_proof_challenge_with_context(
                    request_id,
                    &IssueProofChallengeRequest {
                        provider_id: candidate.provider_id.clone(),
                        device_id: candidate.device_id.clone(),
                        session_id: candidate.session_id.clone(),
                        profile_version: DEFAULT_PROFILE_VERSION.to_string(),
                        required_fingerprint: candidate.hardware_fingerprint.clone(),
                        required_gpu_uuid: candidate.latest_gpu_uuid.clone(),
                        required_backend: DEFAULT_REQUIRED_BACKEND.to_string(),
                        model_artifact_hash: DEFAULT_MODEL_ARTIFACT_HASH.to_string(),
                        prompt_seed,
                        required_proofs: Vec::new(),
                        min_tokens_per_second: 0.0,
                        max_ttft_ms: 0,
                        expires_in_seconds: None,
                    },
                    proof_policy,
                    Some(VerificationChallengeContext {
                        reason: reason.clone(),
                        retry_budget: verification_policy.retry_budget,
                    }),
                )
                .await
            {
                Ok(response) => response,
                Err(SessionError::Conflict(_)) | Err(SessionError::NotFound(_)) => continue,
                Err(error) => return Err(error),
            };
            issued.push(VerificationSweepIssuedChallenge {
                provider_id: response.challenge.provider_id,
                device_id: response.challenge.device_id,
                session_id: response.challenge.session_id,
                challenge_id: response.challenge.challenge_id,
                reason: reason.clone(),
            });
        }

        Ok(RunVerificationSweepResponse {
            request_id: request_id.to_string(),
            evaluated,
            issued,
        })
    }

    pub async fn list_verification_states(
        &self,
        request_id: &str,
        provider_id: &str,
    ) -> Result<ListVerificationStatesResponse, SessionError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT provider_id, device_id, status, policy_version, reason, risk_score, success_count, failure_count, retry_budget_remaining, last_challenge_id, last_verified_challenge_id, last_verified_at, last_failed_at, last_failure_reason, next_due_at, quarantined_at, blocked_at, updated_at FROM provider_verification_states WHERE provider_id = $1 ORDER BY updated_at DESC, device_id",
                &[&provider_id],
            )
            .await?;
        Ok(ListVerificationStatesResponse {
            request_id: request_id.to_string(),
            states: rows.into_iter().map(verification_state_from_row).collect(),
        })
    }

    pub(crate) async fn record_proof_challenge_outcome(
        &self,
        transaction: &Transaction<'_>,
        request_id: &str,
        challenge: &ProofCapabilityChallenge,
        verification: &ProofChallengeVerification,
        accepted: bool,
        policy: VerificationPolicy,
        now: &str,
    ) -> Result<(), SessionError> {
        if accepted {
            record_successful_verification(transaction, request_id, challenge, policy, now).await
        } else {
            let reason = truncate_reason(&verification.errors.join("; "));
            record_failed_verification(transaction, request_id, challenge, policy, &reason, now)
                .await
        }
    }

    async fn verification_candidates(
        &self,
        limit: u32,
    ) -> Result<Vec<VerificationCandidate>, SessionError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT s.provider_id, s.device_id, s.session_id, s.hardware_fingerprint, latest_gpu.gpu_uuid AS latest_gpu_uuid, vs.status AS verification_status, vs.next_due_at, EXISTS (SELECT 1 FROM proof_challenges pc WHERE pc.session_id = s.session_id AND pc.status IN ('issued', 'acknowledged', 'running')) AS has_active_challenge FROM provider_sessions s JOIN providers p ON p.provider_id = s.provider_id JOIN devices d ON d.device_id = s.device_id LEFT JOIN provider_verification_states vs ON vs.provider_id = s.provider_id AND vs.device_id = s.device_id LEFT JOIN LATERAL (SELECT gpu_uuid FROM gpu_telemetry_samples g WHERE g.session_id = s.session_id ORDER BY g.server_received_at DESC LIMIT 1) latest_gpu ON TRUE WHERE s.status IN ('online', 'degraded') AND s.hardware_fingerprint IS NOT NULL AND p.status NOT IN ('blocked', 'quarantined') AND d.status = 'active' ORDER BY COALESCE(vs.next_due_at, s.started_at) ASC LIMIT $1",
                &[&(limit as i64)],
            )
            .await?;
        Ok(rows.into_iter().map(candidate_from_row).collect())
    }

    async fn expire_stale_verification_challenges(
        &self,
        request_id: &str,
        policy: VerificationPolicy,
    ) -> Result<(), SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        transaction
            .execute(
                "UPDATE proof_challenges SET status = 'expired', expired_at = COALESCE(expired_at, $1) WHERE status IN ('issued', 'acknowledged', 'running') AND expires_at <= $1",
                &[&now],
            )
            .await?;
        let rows = transaction
            .query(
                "SELECT pc.challenge_id, pc.provider_id, pc.device_id, pc.session_id, pc.schema_version, pc.nonce, pc.profile_version, pc.required_fingerprint, pc.required_gpu_uuid, pc.required_backend, pc.model_artifact_hash, pc.prompt_seed, pc.required_proofs_json, pc.min_tokens_per_second, pc.max_ttft_ms, pc.issued_at, pc.expires_at FROM provider_verification_states vs JOIN proof_challenges pc ON pc.challenge_id = vs.last_challenge_id WHERE vs.status = 'verification_running' AND pc.status = 'expired' FOR UPDATE",
                &[],
            )
            .await?;
        for row in rows {
            let challenge = challenge_from_expired_row(row)?;
            record_failed_verification(
                &transaction,
                request_id,
                &challenge,
                policy,
                "challenge_expired",
                &now,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

pub(crate) async fn record_challenge_issued_in_transaction(
    transaction: &Transaction<'_>,
    request_id: &str,
    challenge: &ProofCapabilityChallenge,
    context: &VerificationChallengeContext,
) -> Result<(), SessionError> {
    let now = Utc::now().to_rfc3339();
    transaction
        .execute(
            "UPDATE proof_challenges SET trigger_reason = $1, risk_reasons_json = $2, verification_policy_version = $3 WHERE challenge_id = $4",
            &[
                &Some(context.reason.clone()),
                &Some(serde_json::json!([context.reason.as_str()]).to_string()),
                &Some(VERIFICATION_POLICY_VERSION.to_string()),
                &challenge.challenge_id,
            ],
        )
        .await?;
    transaction
        .execute(
            "INSERT INTO provider_verification_states (provider_id, device_id, status, policy_version, reason, risk_score, success_count, failure_count, retry_budget_remaining, last_challenge_id, next_due_at, created_at, updated_at) VALUES ($1, $2, 'verification_running', $3, $4, 0, 0, 0, $5, $6, NULL, $7, $7) ON CONFLICT (provider_id, device_id) DO UPDATE SET status = 'verification_running', policy_version = EXCLUDED.policy_version, reason = EXCLUDED.reason, retry_budget_remaining = GREATEST(provider_verification_states.retry_budget_remaining, EXCLUDED.retry_budget_remaining), last_challenge_id = EXCLUDED.last_challenge_id, next_due_at = NULL, updated_at = EXCLUDED.updated_at",
            &[
                &challenge.provider_id,
                &challenge.device_id,
                &VERIFICATION_POLICY_VERSION,
                &Some(context.reason.clone()),
                &(context.retry_budget as i32),
                &Some(challenge.challenge_id.clone()),
                &now,
            ],
        )
        .await?;
    let metadata = serde_json::json!({
        "challenge_id": challenge.challenge_id,
        "session_id": challenge.session_id,
        "reason": context.reason.as_str(),
        "policy_version": VERIFICATION_POLICY_VERSION,
    })
    .to_string();
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id,
            actor_type: "system",
            actor_id: None,
            entity_type: "provider_verification_state",
            entity_id: &challenge.provider_id,
            event_type: "verification.challenge_issued",
            idempotency_key: None,
            summary: "recurring proof-of-capability challenge issued by verification policy",
            metadata_json: &metadata,
        },
    )
    .await?;
    Ok(())
}

async fn record_successful_verification(
    transaction: &Transaction<'_>,
    request_id: &str,
    challenge: &ProofCapabilityChallenge,
    policy: VerificationPolicy,
    now: &str,
) -> Result<(), SessionError> {
    let next_due_at =
        (parse_timestamp(now)? + Duration::seconds(i64::from(policy.period_seconds))).to_rfc3339();
    transaction
        .execute(
            "INSERT INTO provider_verification_states (provider_id, device_id, status, policy_version, reason, risk_score, success_count, failure_count, retry_budget_remaining, last_challenge_id, last_verified_challenge_id, last_verified_at, next_due_at, created_at, updated_at) VALUES ($1, $2, 'verified', $3, NULL, 0, 1, 0, $4, $5, $5, $6, $7, $6, $6) ON CONFLICT (provider_id, device_id) DO UPDATE SET status = 'verified', policy_version = EXCLUDED.policy_version, reason = NULL, risk_score = GREATEST(0, provider_verification_states.risk_score - 0.2), success_count = provider_verification_states.success_count + 1, failure_count = 0, retry_budget_remaining = EXCLUDED.retry_budget_remaining, last_challenge_id = EXCLUDED.last_challenge_id, last_verified_challenge_id = EXCLUDED.last_verified_challenge_id, last_verified_at = EXCLUDED.last_verified_at, last_failure_reason = NULL, next_due_at = EXCLUDED.next_due_at, updated_at = EXCLUDED.updated_at",
            &[
                &challenge.provider_id,
                &challenge.device_id,
                &VERIFICATION_POLICY_VERSION,
                &(policy.retry_budget as i32),
                &Some(challenge.challenge_id.clone()),
                &now,
                &Some(next_due_at.clone()),
            ],
        )
        .await?;
    let metadata = serde_json::json!({
        "challenge_id": challenge.challenge_id,
        "session_id": challenge.session_id,
        "next_due_at": next_due_at,
        "policy_version": VERIFICATION_POLICY_VERSION,
    })
    .to_string();
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id,
            actor_type: "system",
            actor_id: None,
            entity_type: "provider_verification_state",
            entity_id: &challenge.provider_id,
            event_type: "verification.verified",
            idempotency_key: None,
            summary: "provider verification state marked verified after proof challenge",
            metadata_json: &metadata,
        },
    )
    .await?;
    Ok(())
}

async fn record_failed_verification(
    transaction: &Transaction<'_>,
    request_id: &str,
    challenge: &ProofCapabilityChallenge,
    policy: VerificationPolicy,
    reason: &str,
    now: &str,
) -> Result<(), SessionError> {
    let existing = transaction
        .query_opt(
            "SELECT failure_count, retry_budget_remaining FROM provider_verification_states WHERE provider_id = $1 AND device_id = $2 FOR UPDATE",
            &[&challenge.provider_id, &challenge.device_id],
        )
        .await?;
    let (failure_count, retry_budget_remaining) = existing
        .as_ref()
        .map(|row| {
            (
                row.get::<_, i32>("failure_count"),
                row.get::<_, i32>("retry_budget_remaining"),
            )
        })
        .unwrap_or((0, policy.retry_budget as i32));
    let transition = failed_transition(
        failure_count,
        retry_budget_remaining,
        policy.suspect_failures,
        now,
    );
    transaction
        .execute(
            "INSERT INTO provider_verification_states (provider_id, device_id, status, policy_version, reason, risk_score, success_count, failure_count, retry_budget_remaining, last_challenge_id, last_failed_at, last_failure_reason, next_due_at, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 0.25, 0, 1, $6, $7, $8, $5, $9, $8, $8) ON CONFLICT (provider_id, device_id) DO UPDATE SET status = EXCLUDED.status, policy_version = EXCLUDED.policy_version, reason = EXCLUDED.reason, risk_score = LEAST(1, provider_verification_states.risk_score + 0.25), failure_count = provider_verification_states.failure_count + 1, retry_budget_remaining = EXCLUDED.retry_budget_remaining, last_challenge_id = EXCLUDED.last_challenge_id, last_failed_at = EXCLUDED.last_failed_at, last_failure_reason = EXCLUDED.last_failure_reason, next_due_at = EXCLUDED.next_due_at, updated_at = EXCLUDED.updated_at",
            &[
                &challenge.provider_id,
                &challenge.device_id,
                &transition.status,
                &VERIFICATION_POLICY_VERSION,
                &Some(reason.to_string()),
                &transition.retry_budget_remaining,
                &Some(challenge.challenge_id.clone()),
                &now,
                &transition.next_due_at,
            ],
        )
        .await?;
    let metadata = serde_json::json!({
        "challenge_id": challenge.challenge_id,
        "session_id": challenge.session_id,
        "status": transition.status,
        "reason": reason,
        "retry_budget_remaining": transition.retry_budget_remaining,
        "policy_version": VERIFICATION_POLICY_VERSION,
    })
    .to_string();
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id,
            actor_type: "system",
            actor_id: None,
            entity_type: "provider_verification_state",
            entity_id: &challenge.provider_id,
            event_type: "verification.failed",
            idempotency_key: None,
            summary: "provider verification state updated after failed proof challenge",
            metadata_json: &metadata,
        },
    )
    .await?;
    Ok(())
}

pub(crate) fn failed_transition(
    previous_failure_count: i32,
    retry_budget_remaining: i32,
    suspect_failures: u32,
    now: &str,
) -> FailureTransition {
    let new_failure_count = previous_failure_count.saturating_add(1);
    if new_failure_count >= suspect_failures as i32 {
        return FailureTransition {
            status: "suspect",
            retry_budget_remaining: 0,
            next_due_at: None,
        };
    }
    FailureTransition {
        status: "verification_due",
        retry_budget_remaining: retry_budget_remaining.saturating_sub(1),
        next_due_at: Some(now.to_string()),
    }
}

fn should_issue_challenge(
    candidate: &VerificationCandidate,
    now: DateTime<Utc>,
    force: bool,
) -> bool {
    if candidate.has_active_challenge {
        return false;
    }
    match candidate.state_status.as_deref() {
        Some("blocked" | "quarantined") => false,
        _ if force => true,
        None => true,
        Some("new_provider" | "verification_due" | "suspect") => true,
        Some("verification_running") => true,
        Some("verified") => candidate
            .next_due_at
            .as_deref()
            .map(|raw| due_at_or_past(raw, now))
            .unwrap_or(true),
        Some(_) => false,
    }
}

fn due_at_or_past(raw: &str, now: DateTime<Utc>) -> bool {
    parse_timestamp(raw)
        .map(|due_at| due_at <= now)
        .unwrap_or(true)
}

fn sweep_reason(request: &RunVerificationSweepRequest) -> String {
    request
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or(if request.force {
            "manual_force"
        } else {
            "periodic_due"
        })
        .to_string()
}

fn validate_sweep_request(request: &RunVerificationSweepRequest) -> Result<(), SessionError> {
    if let Some(reason) = request.reason.as_deref()
        && !is_bounded_ascii(reason, 96)
    {
        return Err(SessionError::Invalid(
            "verification sweep reason must be short printable ASCII".to_string(),
        ));
    }
    Ok(())
}

fn truncate_reason(reason: &str) -> String {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        return "verification_failed".to_string();
    }
    trimmed.chars().take(512).collect()
}

fn candidate_from_row(row: Row) -> VerificationCandidate {
    VerificationCandidate {
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        hardware_fingerprint: row.get("hardware_fingerprint"),
        latest_gpu_uuid: row.get("latest_gpu_uuid"),
        state_status: row.get("verification_status"),
        next_due_at: row.get("next_due_at"),
        has_active_challenge: row.get("has_active_challenge"),
    }
}

fn verification_state_from_row(row: Row) -> VerificationStateRecord {
    VerificationStateRecord {
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        status: row.get("status"),
        policy_version: row.get("policy_version"),
        reason: row.get("reason"),
        risk_score: row.get("risk_score"),
        success_count: row.get("success_count"),
        failure_count: row.get("failure_count"),
        retry_budget_remaining: row.get("retry_budget_remaining"),
        last_challenge_id: row.get("last_challenge_id"),
        last_verified_challenge_id: row.get("last_verified_challenge_id"),
        last_verified_at: row.get("last_verified_at"),
        last_failed_at: row.get("last_failed_at"),
        last_failure_reason: row.get("last_failure_reason"),
        next_due_at: row.get("next_due_at"),
        quarantined_at: row.get("quarantined_at"),
        blocked_at: row.get("blocked_at"),
        updated_at: row.get("updated_at"),
    }
}

fn challenge_from_expired_row(row: Row) -> Result<ProofCapabilityChallenge, SessionError> {
    let required_proofs_json: String = row.get("required_proofs_json");
    let required_proofs: Vec<String> = serde_json::from_str(&required_proofs_json)
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    Ok(ProofCapabilityChallenge {
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
        issued_at: row.get("issued_at"),
        expires_at: row.get("expires_at"),
    })
}

fn parse_timestamp(raw: &str) -> Result<DateTime<Utc>, SessionError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))
}

fn is_bounded_ascii(value: &str, max_len: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= max_len
        && trimmed
            .chars()
            .all(|character| character.is_ascii() && !character.is_ascii_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_transition_spends_retries_before_suspect() {
        let transition = failed_transition(0, 2, 3, "2026-07-10T00:00:00Z");
        assert_eq!(transition.status, "verification_due");
        assert_eq!(transition.retry_budget_remaining, 1);
        assert_eq!(
            transition.next_due_at.as_deref(),
            Some("2026-07-10T00:00:00Z")
        );
    }

    #[test]
    fn failed_transition_marks_suspect_at_threshold() {
        let transition = failed_transition(2, 1, 3, "2026-07-10T00:00:00Z");
        assert_eq!(transition.status, "suspect");
        assert_eq!(transition.retry_budget_remaining, 0);
        assert!(transition.next_due_at.is_none());
    }

    #[test]
    fn due_check_fails_open_on_bad_timestamp() {
        assert!(due_at_or_past("not-a-date", Utc::now()));
    }
}
