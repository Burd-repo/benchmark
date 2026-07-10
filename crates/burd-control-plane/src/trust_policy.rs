use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::SessionError;
use burd_protocol::{
    AntifraudEventRecord, ListAntifraudEventsResponse, ListProviderTrustStatesResponse,
    ProviderTrustStateRecord, RunTrustSweepRequest, RunTrustSweepResponse, TRUST_POLICY_VERSION,
    TrustSweepUpdatedState,
};
use chrono::Utc;
use tokio_postgres::Row;
use uuid::Uuid;

const TRUST_SWEEP_LIMIT: u32 = 100;

#[derive(Debug, Clone)]
struct TrustCandidate {
    provider_id: String,
    provider_status: String,
    device_id: String,
    device_status: String,
    session_status: Option<String>,
    heartbeat_count: u32,
    sequence_gap_sum: u32,
    hardware_fingerprint: Option<String>,
    latest_gpu_uuid: Option<String>,
    telemetry_count: u32,
    evidence_count: u32,
    verification_status: Option<String>,
    verification_success_count: u32,
    verification_failure_count: u32,
    verification_risk_score: f64,
    remote_network_score: Option<f64>,
    successful_challenge_count: u32,
    failed_challenge_count: u32,
    same_gpu_provider_count: u32,
    same_fingerprint_device_count: u32,
}

#[derive(Debug, Clone)]
struct TrustEvaluation {
    status: String,
    trust_score: f64,
    risk_score: f64,
    reliability_score: f64,
    reason_codes: Vec<String>,
    antifraud_signals: Vec<AntifraudSignal>,
}

#[derive(Debug, Clone)]
struct AntifraudSignal {
    event_type: &'static str,
    severity: &'static str,
    reason: &'static str,
    metadata: serde_json::Value,
}

impl Database {
    pub async fn run_trust_sweep(
        &self,
        request_id: &str,
        request: &RunTrustSweepRequest,
    ) -> Result<RunTrustSweepResponse, SessionError> {
        validate_trust_sweep_request(request)?;
        let limit = request
            .limit
            .unwrap_or(TRUST_SWEEP_LIMIT)
            .min(TRUST_SWEEP_LIMIT);
        let candidates = self.trust_candidates(limit).await?;
        let evaluated = candidates.len() as u32;
        let mut updated = Vec::with_capacity(candidates.len());

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        for candidate in candidates {
            let evaluation = evaluate_trust(&candidate);
            let now = Utc::now().to_rfc3339();
            let reason_codes_json = serde_json::to_string(&evaluation.reason_codes)
                .map_err(|error| SessionError::Invalid(error.to_string()))?;
            transaction
                .execute(
                    "INSERT INTO provider_trust_states (provider_id, device_id, status, policy_version, trust_score, risk_score, reliability_score, verification_status, remote_network_score, evidence_count, successful_challenge_count, failed_challenge_count, session_status, latest_gpu_uuid, hardware_fingerprint, reason_codes_json, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $17) ON CONFLICT (provider_id, device_id) DO UPDATE SET status = EXCLUDED.status, policy_version = EXCLUDED.policy_version, trust_score = EXCLUDED.trust_score, risk_score = EXCLUDED.risk_score, reliability_score = EXCLUDED.reliability_score, verification_status = EXCLUDED.verification_status, remote_network_score = EXCLUDED.remote_network_score, evidence_count = EXCLUDED.evidence_count, successful_challenge_count = EXCLUDED.successful_challenge_count, failed_challenge_count = EXCLUDED.failed_challenge_count, session_status = EXCLUDED.session_status, latest_gpu_uuid = EXCLUDED.latest_gpu_uuid, hardware_fingerprint = EXCLUDED.hardware_fingerprint, reason_codes_json = EXCLUDED.reason_codes_json, updated_at = EXCLUDED.updated_at",
                    &[
                        &candidate.provider_id,
                        &candidate.device_id,
                        &evaluation.status,
                        &TRUST_POLICY_VERSION,
                        &evaluation.trust_score,
                        &evaluation.risk_score,
                        &Some(evaluation.reliability_score),
                        &candidate.verification_status,
                        &candidate.remote_network_score,
                        &(candidate.evidence_count as i32),
                        &(candidate.successful_challenge_count as i32),
                        &(candidate.failed_challenge_count as i32),
                        &candidate.session_status,
                        &candidate.latest_gpu_uuid,
                        &candidate.hardware_fingerprint,
                        &reason_codes_json,
                        &now,
                    ],
                )
                .await?;
            for signal in &evaluation.antifraud_signals {
                record_antifraud_signal(
                    &transaction,
                    &candidate.provider_id,
                    &candidate.device_id,
                    signal,
                    &now,
                )
                .await?;
            }
            let audit_metadata = serde_json::json!({
                "device_id": candidate.device_id,
                "status": evaluation.status,
                "trust_score": evaluation.trust_score,
                "risk_score": evaluation.risk_score,
                "reason_codes": evaluation.reason_codes,
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "system",
                    actor_id: None,
                    entity_type: "provider_trust_state",
                    entity_id: &candidate.provider_id,
                    event_type: "trust_state.recalculated",
                    idempotency_key: None,
                    summary: "backend trust and antifraud state recalculated",
                    metadata_json: &audit_metadata,
                },
            )
            .await?;
            updated.push(TrustSweepUpdatedState {
                provider_id: candidate.provider_id,
                device_id: candidate.device_id,
                status: evaluation.status,
                trust_score: evaluation.trust_score,
                risk_score: evaluation.risk_score,
                reason_codes: evaluation.reason_codes,
            });
        }
        transaction.commit().await?;

        Ok(RunTrustSweepResponse {
            request_id: request_id.to_string(),
            evaluated,
            updated,
        })
    }

    pub async fn list_provider_trust_states(
        &self,
        request_id: &str,
        provider_id: &str,
    ) -> Result<ListProviderTrustStatesResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT provider_id, device_id, status, policy_version, trust_score, risk_score, reliability_score, verification_status, remote_network_score, evidence_count, successful_challenge_count, failed_challenge_count, session_status, latest_gpu_uuid, hardware_fingerprint, reason_codes_json, created_at, updated_at FROM provider_trust_states WHERE provider_id = $1 ORDER BY trust_score DESC, updated_at DESC",
                &[&provider_id],
            )
            .await?;
        let states = rows
            .into_iter()
            .map(trust_state_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListProviderTrustStatesResponse {
            request_id: request_id.to_string(),
            states,
        })
    }

    pub async fn list_antifraud_events(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListAntifraudEventsResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let limit = limit.clamp(1, 200) as i64;
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT event_id, provider_id, device_id, event_type, severity, status, reason, metadata_json, first_seen_at, last_seen_at, occurrence_count FROM antifraud_events WHERE provider_id = $1 ORDER BY last_seen_at DESC, severity DESC LIMIT $2",
                &[&provider_id, &limit],
            )
            .await?;
        let events = rows
            .into_iter()
            .map(antifraud_event_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListAntifraudEventsResponse {
            request_id: request_id.to_string(),
            events,
        })
    }

    async fn trust_candidates(&self, limit: u32) -> Result<Vec<TrustCandidate>, SessionError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT p.provider_id, p.status AS provider_status, d.device_id, d.status AS device_status, s.status AS session_status, COALESCE(hb.heartbeat_count, 0) AS heartbeat_count, COALESCE(hb.sequence_gap_sum, 0) AS sequence_gap_sum, s.hardware_fingerprint, latest_gpu.gpu_uuid AS latest_gpu_uuid, COALESCE(telemetry.telemetry_count, 0) AS telemetry_count, COALESCE(evidence.evidence_count, 0) AS evidence_count, vs.status AS verification_status, COALESCE(vs.success_count, 0) AS verification_success_count, COALESCE(vs.failure_count, 0) AS verification_failure_count, COALESCE(vs.risk_score, 0) AS verification_risk_score, ns.remote_network_score, COALESCE(challenges.successful_challenge_count, 0) AS successful_challenge_count, COALESCE(challenges.failed_challenge_count, 0) AS failed_challenge_count, COALESCE(gpu_dupes.provider_count, 0) AS same_gpu_provider_count, COALESCE(fp_dupes.device_count, 0) AS same_fingerprint_device_count FROM devices d JOIN providers p ON p.provider_id = d.provider_id LEFT JOIN LATERAL (SELECT session_id, status, sequence_last, hardware_fingerprint, last_seen_at, started_at FROM provider_sessions s WHERE s.provider_id = d.provider_id AND s.device_id = d.device_id ORDER BY started_at DESC LIMIT 1) s ON TRUE LEFT JOIN LATERAL (SELECT COUNT(*)::BIGINT AS heartbeat_count, COALESCE(SUM(sequence_gap), 0)::BIGINT AS sequence_gap_sum FROM session_heartbeats h WHERE h.session_id = s.session_id) hb ON TRUE LEFT JOIN LATERAL (SELECT COUNT(*)::BIGINT AS telemetry_count FROM telemetry_batches t WHERE t.provider_id = d.provider_id AND t.device_id = d.device_id) telemetry ON TRUE LEFT JOIN LATERAL (SELECT gpu_uuid FROM gpu_telemetry_samples g WHERE g.provider_id = d.provider_id AND g.device_id = d.device_id ORDER BY server_received_at DESC LIMIT 1) latest_gpu ON TRUE LEFT JOIN LATERAL (SELECT COUNT(*)::BIGINT AS evidence_count FROM evidence_records e WHERE e.provider_id = d.provider_id AND e.device_id = d.device_id AND e.status = 'valid') evidence ON TRUE LEFT JOIN provider_verification_states vs ON vs.provider_id = d.provider_id AND vs.device_id = d.device_id LEFT JOIN provider_network_states ns ON ns.provider_id = d.provider_id AND ns.device_id = d.device_id LEFT JOIN LATERAL (SELECT COUNT(*) FILTER (WHERE status = 'verified')::BIGINT AS successful_challenge_count, COUNT(*) FILTER (WHERE status = 'failed')::BIGINT AS failed_challenge_count FROM proof_challenges pc WHERE pc.provider_id = d.provider_id AND pc.device_id = d.device_id) challenges ON TRUE LEFT JOIN LATERAL (SELECT COUNT(DISTINCT g2.provider_id)::BIGINT AS provider_count FROM gpu_telemetry_samples g2 WHERE latest_gpu.gpu_uuid IS NOT NULL AND g2.gpu_uuid = latest_gpu.gpu_uuid) gpu_dupes ON TRUE LEFT JOIN LATERAL (SELECT COUNT(DISTINCT sf.device_id)::BIGINT AS device_count FROM provider_sessions sf WHERE s.hardware_fingerprint IS NOT NULL AND sf.hardware_fingerprint = s.hardware_fingerprint) fp_dupes ON TRUE WHERE d.status IN ('active', 'revoked') ORDER BY d.updated_at DESC LIMIT $1",
                &[&(limit as i64)],
            )
            .await?;
        Ok(rows.into_iter().map(candidate_from_row).collect())
    }
}

async fn record_antifraud_signal(
    transaction: &tokio_postgres::Transaction<'_>,
    provider_id: &str,
    device_id: &str,
    signal: &AntifraudSignal,
    now: &str,
) -> Result<(), SessionError> {
    let event_id = format!("antifraud_{}", Uuid::new_v4());
    let metadata_json = serde_json::to_string(&signal.metadata)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    transaction
        .execute(
            "INSERT INTO antifraud_events (event_id, provider_id, device_id, event_type, severity, status, reason, metadata_json, first_seen_at, last_seen_at, occurrence_count) VALUES ($1, $2, $3, $4, $5, 'active', $6, $7, $8, $8, 1) ON CONFLICT (provider_id, device_id, event_type, reason) DO UPDATE SET severity = EXCLUDED.severity, status = 'active', metadata_json = EXCLUDED.metadata_json, last_seen_at = EXCLUDED.last_seen_at, occurrence_count = antifraud_events.occurrence_count + 1",
            &[
                &event_id,
                &provider_id,
                &device_id,
                &signal.event_type,
                &signal.severity,
                &signal.reason,
                &metadata_json,
                &now,
            ],
        )
        .await?;
    Ok(())
}

fn evaluate_trust(candidate: &TrustCandidate) -> TrustEvaluation {
    let mut reason_codes = Vec::new();
    let mut signals = Vec::new();
    let reliability_score = backend_reliability_score(candidate);
    let mut risk_score = 0.0;
    let mut trust_score = 45.0 + reliability_score * 0.25;

    match candidate.verification_status.as_deref() {
        Some("verified") => {
            trust_score += 20.0;
            reason_codes.push("verification_verified".to_string());
        }
        Some("verification_running") => {
            trust_score += 5.0;
            risk_score += 5.0;
            reason_codes.push("verification_running".to_string());
        }
        Some("verification_due") => {
            risk_score += 10.0;
            reason_codes.push("verification_due".to_string());
        }
        Some("suspect") => {
            trust_score -= 25.0;
            risk_score += 35.0;
            push_signal(
                &mut signals,
                "verification_suspect",
                "high",
                "verification_state_suspect",
                serde_json::json!({"failure_count": candidate.verification_failure_count}),
            );
        }
        Some("quarantined") => {
            trust_score -= 50.0;
            risk_score += 80.0;
            reason_codes.push("verification_quarantined".to_string());
        }
        Some("blocked") => {
            trust_score -= 75.0;
            risk_score += 100.0;
            reason_codes.push("verification_blocked".to_string());
        }
        _ => reason_codes.push("verification_missing".to_string()),
    }

    trust_score += (candidate.evidence_count.min(4) as f64) * 2.5;
    trust_score += (candidate.successful_challenge_count.min(3) as f64) * 5.0;
    trust_score += (candidate.verification_success_count.min(3) as f64) * 2.0;
    trust_score -= (candidate.failed_challenge_count.min(4) as f64) * 8.0;
    risk_score += (candidate.failed_challenge_count.min(4) as f64) * 15.0;
    risk_score += (candidate.verification_risk_score * 100.0).clamp(0.0, 25.0);

    if candidate.evidence_count == 0 {
        risk_score += 8.0;
        push_signal(
            &mut signals,
            "missing_evidence",
            "low",
            "no_valid_remote_evidence",
            serde_json::json!({}),
        );
    }
    if candidate.successful_challenge_count == 0 {
        risk_score += 8.0;
        push_signal(
            &mut signals,
            "missing_capability_proof",
            "low",
            "no_successful_remote_challenge",
            serde_json::json!({}),
        );
    }
    if candidate.failed_challenge_count > 0 {
        push_signal(
            &mut signals,
            "challenge_failures",
            "medium",
            "failed_remote_challenges",
            serde_json::json!({"failed_challenge_count": candidate.failed_challenge_count}),
        );
    }

    match candidate.session_status.as_deref() {
        Some("online") => reason_codes.push("session_online".to_string()),
        Some("degraded") => {
            trust_score -= 8.0;
            risk_score += 15.0;
            push_signal(
                &mut signals,
                "degraded_session",
                "medium",
                "latest_session_degraded",
                serde_json::json!({"sequence_gap_sum": candidate.sequence_gap_sum}),
            );
        }
        Some("offline") => {
            trust_score -= 12.0;
            risk_score += 20.0;
            reason_codes.push("session_offline".to_string());
        }
        Some("expired" | "revoked") => {
            trust_score -= 20.0;
            risk_score += 30.0;
            reason_codes.push("session_terminal".to_string());
        }
        _ => {
            trust_score -= 10.0;
            risk_score += 10.0;
            reason_codes.push("session_missing".to_string());
        }
    }

    if candidate.heartbeat_count > 0 && candidate.telemetry_count == 0 {
        trust_score -= 8.0;
        risk_score += 15.0;
        push_signal(
            &mut signals,
            "heartbeat_without_telemetry",
            "medium",
            "heartbeat_seen_without_gpu_telemetry",
            serde_json::json!({"heartbeat_count": candidate.heartbeat_count}),
        );
    }
    if candidate.telemetry_count > 0 {
        trust_score += 5.0;
        reason_codes.push("telemetry_present".to_string());
    }

    if let Some(network_score) = candidate.remote_network_score {
        trust_score += ((network_score - 50.0) * 0.2).clamp(-10.0, 10.0);
        if network_score < 40.0 {
            risk_score += 20.0;
            push_signal(
                &mut signals,
                "weak_remote_network",
                "medium",
                "remote_network_score_below_40",
                serde_json::json!({"remote_network_score": network_score}),
            );
        } else if network_score < 60.0 {
            risk_score += 10.0;
            reason_codes.push("network_degraded".to_string());
        }
    } else {
        reason_codes.push("network_unobserved".to_string());
    }

    if candidate.same_gpu_provider_count > 1 {
        trust_score -= 25.0;
        risk_score += 70.0;
        push_signal(
            &mut signals,
            "duplicate_gpu_uuid",
            "high",
            "same_gpu_uuid_seen_on_multiple_providers",
            serde_json::json!({
                "gpu_uuid": candidate.latest_gpu_uuid,
                "provider_count": candidate.same_gpu_provider_count,
            }),
        );
    }
    if candidate.same_fingerprint_device_count > 1 {
        trust_score -= 20.0;
        risk_score += 40.0;
        push_signal(
            &mut signals,
            "fingerprint_reuse",
            "high",
            "same_hardware_fingerprint_seen_on_multiple_devices",
            serde_json::json!({
                "hardware_fingerprint": candidate.hardware_fingerprint,
                "device_count": candidate.same_fingerprint_device_count,
            }),
        );
    }
    if matches!(candidate.provider_status.as_str(), "blocked")
        || candidate.device_status != "active"
    {
        trust_score = 0.0;
        risk_score = 100.0;
        reason_codes.push("provider_or_device_blocked".to_string());
    } else if matches!(candidate.provider_status.as_str(), "quarantined") {
        trust_score -= 50.0;
        risk_score += 80.0;
        reason_codes.push("provider_quarantined".to_string());
    }

    let risk_score = round_score(risk_score.clamp(0.0, 100.0));
    let trust_score = round_score((trust_score - risk_score * 0.35).clamp(0.0, 100.0));
    let status = trust_status(candidate, trust_score, risk_score);
    if !reason_codes.iter().any(|reason| reason == &status) {
        reason_codes.push(status.clone());
    }
    reason_codes.sort();
    reason_codes.dedup();

    TrustEvaluation {
        status,
        trust_score,
        risk_score,
        reliability_score,
        reason_codes,
        antifraud_signals: signals,
    }
}

fn backend_reliability_score(candidate: &TrustCandidate) -> f64 {
    let base = match candidate.session_status.as_deref() {
        Some("online") => 70.0,
        Some("degraded") => 45.0,
        Some("offline") => 30.0,
        Some("pending_connection") => 25.0,
        Some("expired" | "revoked") => 15.0,
        _ => 10.0,
    };
    let heartbeat_bonus = (candidate.heartbeat_count.min(10) as f64) * 2.0;
    let gap_penalty = (candidate.sequence_gap_sum.min(10) as f64) * 3.0;
    let telemetry_bonus = if candidate.telemetry_count > 0 {
        5.0
    } else {
        0.0
    };
    round_score((base + heartbeat_bonus + telemetry_bonus - gap_penalty).clamp(0.0, 100.0))
}

fn trust_status(candidate: &TrustCandidate, trust_score: f64, risk_score: f64) -> String {
    if matches!(candidate.provider_status.as_str(), "blocked")
        || candidate.device_status != "active"
    {
        return "blocked".to_string();
    }
    if candidate.evidence_count == 0
        && candidate.successful_challenge_count == 0
        && candidate.telemetry_count == 0
        && risk_score < 45.0
    {
        return "new_provider".to_string();
    }
    if risk_score >= 70.0 {
        return "suspect".to_string();
    }
    if risk_score >= 45.0 || trust_score < 45.0 {
        return "degraded".to_string();
    }
    if candidate.evidence_count == 0
        || candidate.successful_challenge_count == 0
        || candidate.verification_status.as_deref() != Some("verified")
    {
        return "insufficient_history".to_string();
    }
    if trust_score >= 85.0 && risk_score < 15.0 {
        "highly_trusted".to_string()
    } else if trust_score >= 65.0 && risk_score < 35.0 {
        "trusted".to_string()
    } else {
        "degraded".to_string()
    }
}

fn push_signal(
    signals: &mut Vec<AntifraudSignal>,
    event_type: &'static str,
    severity: &'static str,
    reason: &'static str,
    metadata: serde_json::Value,
) {
    signals.push(AntifraudSignal {
        event_type,
        severity,
        reason,
        metadata,
    });
}

fn candidate_from_row(row: Row) -> TrustCandidate {
    TrustCandidate {
        provider_id: row.get("provider_id"),
        provider_status: row.get("provider_status"),
        device_id: row.get("device_id"),
        device_status: row.get("device_status"),
        session_status: row.get("session_status"),
        heartbeat_count: row.get::<_, i64>("heartbeat_count").max(0) as u32,
        sequence_gap_sum: row.get::<_, i64>("sequence_gap_sum").max(0) as u32,
        hardware_fingerprint: row.get("hardware_fingerprint"),
        latest_gpu_uuid: row.get("latest_gpu_uuid"),
        telemetry_count: row.get::<_, i64>("telemetry_count").max(0) as u32,
        evidence_count: row.get::<_, i64>("evidence_count").max(0) as u32,
        verification_status: row.get("verification_status"),
        verification_success_count: row.get::<_, i32>("verification_success_count").max(0) as u32,
        verification_failure_count: row.get::<_, i32>("verification_failure_count").max(0) as u32,
        verification_risk_score: row.get("verification_risk_score"),
        remote_network_score: row.get("remote_network_score"),
        successful_challenge_count: row.get::<_, i64>("successful_challenge_count").max(0) as u32,
        failed_challenge_count: row.get::<_, i64>("failed_challenge_count").max(0) as u32,
        same_gpu_provider_count: row.get::<_, i64>("same_gpu_provider_count").max(0) as u32,
        same_fingerprint_device_count: row.get::<_, i64>("same_fingerprint_device_count").max(0)
            as u32,
    }
}

fn trust_state_from_row(row: Row) -> Result<ProviderTrustStateRecord, SessionError> {
    let reason_codes_json: String = row.get("reason_codes_json");
    Ok(ProviderTrustStateRecord {
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        status: row.get("status"),
        policy_version: row.get("policy_version"),
        trust_score: row.get("trust_score"),
        risk_score: row.get("risk_score"),
        reliability_score: row.get("reliability_score"),
        verification_status: row.get("verification_status"),
        remote_network_score: row.get("remote_network_score"),
        evidence_count: row.get::<_, i32>("evidence_count").max(0) as u32,
        successful_challenge_count: row.get::<_, i32>("successful_challenge_count").max(0) as u32,
        failed_challenge_count: row.get::<_, i32>("failed_challenge_count").max(0) as u32,
        session_status: row.get("session_status"),
        latest_gpu_uuid: row.get("latest_gpu_uuid"),
        hardware_fingerprint: row.get("hardware_fingerprint"),
        reason_codes: serde_json::from_str(&reason_codes_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn antifraud_event_from_row(row: Row) -> Result<AntifraudEventRecord, SessionError> {
    let metadata_json: String = row.get("metadata_json");
    Ok(AntifraudEventRecord {
        event_id: row.get("event_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        event_type: row.get("event_type"),
        severity: row.get("severity"),
        status: row.get("status"),
        reason: row.get("reason"),
        metadata: serde_json::from_str(&metadata_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        first_seen_at: row.get("first_seen_at"),
        last_seen_at: row.get("last_seen_at"),
        occurrence_count: row.get::<_, i32>("occurrence_count").max(0) as u32,
    })
}

fn validate_trust_sweep_request(request: &RunTrustSweepRequest) -> Result<(), SessionError> {
    if let Some(reason) = request.reason.as_deref()
        && !is_bounded_ascii(reason, 96)
    {
        return Err(SessionError::Invalid(
            "trust sweep reason must be short printable ASCII".to_string(),
        ));
    }
    Ok(())
}

fn validate_id(label: &str, value: &str, maximum_len: usize) -> Result<(), SessionError> {
    let valid = !value.trim().is_empty()
        && value.len() <= maximum_len
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(SessionError::Invalid(format!("{label} is invalid")))
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

fn round_score(score: f64) -> f64 {
    (score * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate() -> TrustCandidate {
        TrustCandidate {
            provider_id: "provider_1".to_string(),
            provider_status: "available".to_string(),
            device_id: "device_1".to_string(),
            device_status: "active".to_string(),
            session_status: Some("online".to_string()),
            heartbeat_count: 8,
            sequence_gap_sum: 0,
            hardware_fingerprint: Some("fp_1".to_string()),
            latest_gpu_uuid: Some("GPU-1".to_string()),
            telemetry_count: 4,
            evidence_count: 2,
            verification_status: Some("verified".to_string()),
            verification_success_count: 1,
            verification_failure_count: 0,
            verification_risk_score: 0.0,
            remote_network_score: Some(92.0),
            successful_challenge_count: 2,
            failed_challenge_count: 0,
            same_gpu_provider_count: 1,
            same_fingerprint_device_count: 1,
        }
    }

    #[test]
    fn verified_low_risk_provider_becomes_trusted() {
        let evaluation = evaluate_trust(&candidate());
        assert!(matches!(
            evaluation.status.as_str(),
            "trusted" | "highly_trusted"
        ));
        assert!(
            evaluation.trust_score >= 65.0,
            "score={}",
            evaluation.trust_score
        );
        assert!(
            evaluation.risk_score < 35.0,
            "risk={}",
            evaluation.risk_score
        );
    }

    #[test]
    fn duplicate_gpu_marks_provider_suspect() {
        let mut candidate = candidate();
        candidate.same_gpu_provider_count = 2;
        let evaluation = evaluate_trust(&candidate);
        assert_eq!(evaluation.status, "suspect");
        assert!(
            evaluation
                .antifraud_signals
                .iter()
                .any(|signal| signal.event_type == "duplicate_gpu_uuid")
        );
    }

    #[test]
    fn empty_provider_stays_cold_start() {
        let mut candidate = candidate();
        candidate.session_status = None;
        candidate.telemetry_count = 0;
        candidate.evidence_count = 0;
        candidate.verification_status = None;
        candidate.successful_challenge_count = 0;
        candidate.remote_network_score = None;
        let evaluation = evaluate_trust(&candidate);
        assert_eq!(evaluation.status, "new_provider");
        assert!(
            evaluation
                .reason_codes
                .contains(&"verification_missing".to_string())
        );
    }

    #[tokio::test]
    #[ignore]
    async fn persists_trust_state_and_antifraud_events() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        let expires_at = (Utc::now() + chrono::Duration::minutes(30)).to_rfc3339();
        client
            .execute(
                "INSERT INTO providers (provider_id, user_id, display_name, status, created_at, updated_at) VALUES ($1, NULL, $2, 'available', $3, $3)",
                &[&"provider_1", &"Trust Provider", &now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ($1, $2, $3, 'active', $4, $4)",
                &[&"device_1", &"provider_1", &"machine_1", &now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, last_seen_at, expires_at, hardware_fingerprint) VALUES ($1, $2, $3, 'degraded', 2, $4, $4, $5, $6)",
                &[&"session_1", &"provider_1", &"device_1", &now, &expires_at, &"fp_1"],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO session_heartbeats (heartbeat_id, session_id, sequence, client_sent_at, server_received_at, sequence_gap, payload_hash, payload_json) VALUES ($1, $2, 2, $3, $3, 1, $4, $5)",
                &[&"heartbeat_1", &"session_1", &now, &"hash", &"{}"],
            )
            .await
            .unwrap();

        let response = db
            .run_trust_sweep(
                "req_trust",
                &RunTrustSweepRequest {
                    limit: Some(10),
                    force: false,
                    reason: Some("integration".to_string()),
                },
            )
            .await
            .unwrap();
        assert_eq!(response.evaluated, 1);
        assert_eq!(response.updated.len(), 1);

        let states = db
            .list_provider_trust_states("req_list", "provider_1")
            .await
            .unwrap();
        assert_eq!(states.states.len(), 1);
        assert!(states.states[0].risk_score > 0.0);

        let events = db
            .list_antifraud_events("req_events", "provider_1", 50)
            .await
            .unwrap();
        assert!(
            events
                .events
                .iter()
                .any(|event| event.event_type == "heartbeat_without_telemetry")
        );

        db.drop_schema_for_test().await.unwrap();
    }
}
