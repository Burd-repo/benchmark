use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::{AuthorizedSession, SessionError};
use burd_protocol::{
    EVIDENCE_CANONICALIZATION_VERSION, EVIDENCE_REGISTRY_SCHEMA_VERSION, EvidenceRecord,
    EvidenceVerification, KEY_ALGORITHM, ListEvidenceResponse, RevokeEvidenceResponse,
    SIGNED_REPORT_TTL_SECONDS, SignedReport, SubmitEvidenceRequest, SubmitEvidenceResponse,
    evidence_freshness_at, hash_canonical, verify_message,
};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

impl Database {
    pub async fn submit_evidence_record(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        session_id: &str,
        request: &SubmitEvidenceRequest,
        object_storage_dir: &str,
    ) -> Result<SubmitEvidenceResponse, SessionError> {
        if session_id != authorized.session_id
            || request
                .session_id
                .as_deref()
                .is_some_and(|value| value != authorized.session_id)
        {
            return Err(SessionError::Unauthorized);
        }
        validate_evidence_type(&request.evidence_type)?;
        if request
            .subject_id
            .as_deref()
            .is_some_and(|value| !is_safe_identifier(value, 128))
        {
            return Err(SessionError::Invalid(
                "evidence subject_id contains unsupported characters".to_string(),
            ));
        }
        if request.metadata.as_ref().is_some_and(contains_secret_field) {
            return Err(SessionError::Invalid(
                "evidence metadata contains a forbidden secret field".to_string(),
            ));
        }

        let signed = &request.signed_report;
        let evidence_hash = hash_canonical(signed).map_err(SessionError::Invalid)?;
        let computed_report_hash = hash_canonical(&signed.report).map_err(SessionError::Invalid)?;
        let server_now = Utc::now();
        let server_freshness =
            evidence_freshness_at(&signed.signed_at, SIGNED_REPORT_TTL_SECONDS, server_now)
                .map_err(SessionError::Invalid)?;
        let server_received_at = server_now.to_rfc3339();

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let context = load_evidence_context(&transaction, authorized).await?;
        let verification = build_verification(
            signed,
            &evidence_hash,
            &computed_report_hash,
            &server_received_at,
            &server_freshness,
            &authorized.provider_id,
            &context,
        );

        if !verification.errors.is_empty() {
            record_rejected_evidence(
                &transaction,
                request_id,
                authorized,
                &evidence_hash,
                &signed.report_hash,
                &verification,
            )
            .await?;
            transaction.commit().await?;
            if !verification.active_key_bound
                || !verification.provider_bound
                || !verification.device_bound
            {
                return Err(SessionError::Unauthorized);
            }
            if !verification.signature_valid {
                return Err(SessionError::SignatureInvalid);
            }
            return Err(SessionError::Invalid(verification.errors.join("; ")));
        }

        if let Some(existing) = transaction
            .query_opt(
                evidence_select_sql("WHERE evidence_hash = $1"),
                &[&evidence_hash],
            )
            .await?
        {
            let evidence = evidence_record_from_row(existing)?;
            transaction.commit().await?;
            return Ok(SubmitEvidenceResponse {
                request_id: request_id.to_string(),
                duplicate: true,
                evidence,
            });
        }

        let object_key = write_evidence_object(
            object_storage_dir,
            &authorized.provider_id,
            &evidence_hash,
            signed,
        )
        .await?;
        let evidence_id = format!("evidence_{}", Uuid::new_v4());
        let verification_json = serde_json::to_string(&verification)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let status = if server_freshness.is_expired {
            "expired"
        } else {
            "valid"
        };
        let fingerprint = signed.report.hardware_fingerprint.clone();
        transaction
            .execute(
                "INSERT INTO evidence_records (evidence_id, provider_id, device_id, evidence_type, canonicalization_version, evidence_hash, object_key, status, server_received_at, expires_at, verification_json, session_id, public_key_id, report_hash, hardware_fingerprint, signed_at, issued_at, subject_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
                &[
                    &evidence_id,
                    &authorized.provider_id,
                    &Some(authorized.device_id.clone()),
                    &request.evidence_type,
                    &signed.canonicalization_version,
                    &evidence_hash,
                    &Some(object_key.clone()),
                    &status,
                    &server_received_at,
                    &Some(server_freshness.expires_at.clone()),
                    &verification_json,
                    &Some(authorized.session_id.clone()),
                    &Some(context.public_key_id.clone()),
                    &Some(signed.report_hash.clone()),
                    &fingerprint,
                    &Some(signed.signed_at.clone()),
                    &Some(server_freshness.issued_at.clone()),
                    &request.subject_id,
                ],
            )
            .await?;
        insert_hardware_snapshot(
            &transaction,
            authorized,
            signed,
            fingerprint.as_deref(),
            &server_received_at,
        )
        .await?;
        let audit_metadata = serde_json::json!({
            "evidence_hash": evidence_hash,
            "report_hash": signed.report_hash,
            "status": status,
            "object_key": object_key,
            "expired_by_server": server_freshness.is_expired,
            "agent_envelope_claimed_expired": signed.evidence.as_ref().map(|value| value.is_expired),
            "agent_report_claimed_expired": signed.report.evidence.as_ref().map(|value| value.is_expired),
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_key",
                actor_id: Some(context.public_key_id.clone()),
                entity_type: "evidence_record",
                entity_id: &evidence_id,
                event_type: "evidence_record.accepted",
                idempotency_key: None,
                summary: "signed evidence accepted into remote registry",
                metadata_json: &audit_metadata,
            },
        )
        .await?;
        let row = transaction
            .query_one(
                evidence_select_sql("WHERE evidence_id = $1"),
                &[&evidence_id],
            )
            .await?;
        let evidence = evidence_record_from_row(row)?;
        transaction.commit().await?;

        Ok(SubmitEvidenceResponse {
            request_id: request_id.to_string(),
            duplicate: false,
            evidence,
        })
    }

    pub async fn list_provider_evidence_records(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListEvidenceResponse, SessionError> {
        let client = self.connect().await?;
        if client
            .query_opt(
                "SELECT provider_id FROM providers WHERE provider_id = $1",
                &[&provider_id],
            )
            .await?
            .is_none()
        {
            return Err(SessionError::NotFound("provider not found".to_string()));
        }
        let limit = limit.clamp(1, 200) as i64;
        let rows = client
            .query(
                evidence_select_sql(
                    "WHERE provider_id = $1 ORDER BY server_received_at DESC LIMIT $2",
                ),
                &[&provider_id, &limit],
            )
            .await?;
        let records = rows
            .into_iter()
            .map(evidence_record_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListEvidenceResponse {
            request_id: request_id.to_string(),
            provider_id: provider_id.to_string(),
            records,
        })
    }

    pub async fn get_evidence_record(
        &self,
        evidence_id: &str,
    ) -> Result<Option<EvidenceRecord>, SessionError> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                evidence_select_sql("WHERE evidence_id = $1"),
                &[&evidence_id],
            )
            .await?;
        row.map(evidence_record_from_row).transpose()
    }

    pub async fn revoke_evidence_record(
        &self,
        evidence_id: &str,
        request_id: &str,
        reason: &str,
    ) -> Result<RevokeEvidenceResponse, SessionError> {
        validate_revocation_reason(reason)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        let row = transaction
            .query_opt(
                "UPDATE evidence_records SET status = 'revoked', revoked_at = COALESCE(revoked_at, $1), revocation_reason = $2 WHERE evidence_id = $3 RETURNING provider_id",
                &[&now, &reason, &evidence_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("evidence record not found".to_string()))?;
        let provider_id: String = row.get("provider_id");
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "evidence_record",
                entity_id: evidence_id,
                event_type: "evidence_record.revoked",
                idempotency_key: None,
                summary: "remote evidence record revoked",
                metadata_json: &serde_json::json!({
                    "provider_id": provider_id,
                    "reason": reason,
                })
                .to_string(),
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(RevokeEvidenceResponse {
            request_id: request_id.to_string(),
            evidence_id: evidence_id.to_string(),
            status: "revoked".to_string(),
            revoked_at: now,
            reason: reason.to_string(),
        })
    }
}

#[derive(Debug)]
struct EvidenceContext {
    session_status: String,
    session_fingerprint: Option<String>,
    public_key_id: String,
    active_public_key: String,
    machine_id: Option<String>,
    local_provider_id: Option<String>,
    enrolled_fingerprint: Option<String>,
}

async fn load_evidence_context(
    transaction: &Transaction<'_>,
    authorized: &AuthorizedSession,
) -> Result<EvidenceContext, SessionError> {
    let session = transaction
        .query_opt(
            "SELECT status, hardware_fingerprint FROM provider_sessions WHERE session_id = $1 FOR UPDATE",
            &[&authorized.session_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
    let session_status: String = session.get("status");
    if !matches!(
        session_status.as_str(),
        "pending_connection" | "online" | "degraded" | "offline"
    ) {
        return Err(SessionError::Conflict(
            "evidence requires a nonterminal remote session".to_string(),
        ));
    }

    let key = transaction
        .query_opt(
            "SELECT public_key_id, public_key FROM provider_public_keys WHERE provider_id = $1 AND device_id = $2 AND status = 'active'",
            &[&authorized.provider_id, &authorized.device_id],
        )
        .await?
        .ok_or(SessionError::Unauthorized)?;
    let device = transaction
        .query_opt(
            "SELECT d.machine_id, (SELECT e.local_provider_id FROM device_enrollments e WHERE e.device_id = d.device_id AND e.status = 'completed' ORDER BY e.completed_at DESC LIMIT 1) AS local_provider_id, (SELECT e.hardware_fingerprint FROM device_enrollments e WHERE e.device_id = d.device_id AND e.status = 'completed' ORDER BY e.completed_at DESC LIMIT 1) AS enrolled_fingerprint FROM devices d WHERE d.provider_id = $1 AND d.device_id = $2",
            &[&authorized.provider_id, &authorized.device_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("device not found".to_string()))?;

    Ok(EvidenceContext {
        session_status,
        session_fingerprint: session.get("hardware_fingerprint"),
        public_key_id: key.get("public_key_id"),
        active_public_key: key.get("public_key"),
        machine_id: device.get("machine_id"),
        local_provider_id: device.get("local_provider_id"),
        enrolled_fingerprint: device.get("enrolled_fingerprint"),
    })
}

fn build_verification(
    signed: &SignedReport,
    evidence_hash: &str,
    computed_report_hash: &str,
    checked_at: &str,
    server_freshness: &burd_protocol::EvidenceFreshness,
    backend_provider_id: &str,
    context: &EvidenceContext,
) -> EvidenceVerification {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let report_hash_valid = computed_report_hash == signed.report_hash;
    if !report_hash_valid {
        errors.push("report_hash does not match canonical report".to_string());
    }
    let canonicalization_valid =
        signed.canonicalization_version == EVIDENCE_CANONICALIZATION_VERSION;
    if !canonicalization_valid {
        errors.push("unsupported evidence canonicalization version".to_string());
    }
    if signed.key_algorithm != KEY_ALGORITHM {
        errors.push(format!(
            "unsupported key algorithm '{}'",
            signed.key_algorithm
        ));
    }

    let active_key_bound = signed.public_key == context.active_public_key;
    if !active_key_bound {
        errors.push(
            "signed report public key is not the active backend key for this device".to_string(),
        );
    }
    let signature_valid = active_key_bound
        && verify_message(
            &context.active_public_key,
            signed.report_hash.as_bytes(),
            &signed.signature,
        )
        .unwrap_or(false);
    if !signature_valid {
        errors.push("signed report signature invalid for active backend key".to_string());
    }

    let provider_bound = signed.provider_id == backend_provider_id
        || context
            .local_provider_id
            .as_deref()
            .is_some_and(|value| value == signed.provider_id);
    if !provider_bound {
        errors.push("signed report provider_id is not bound to this backend provider".to_string());
    }
    let device_bound = context
        .machine_id
        .as_deref()
        .is_some_and(|value| value == signed.machine_id);
    if !device_bound {
        errors.push("signed report machine_id does not match the enrolled device".to_string());
    }

    let report_fingerprint = signed.report.hardware_fingerprint.as_deref();
    let fingerprint_bound = report_fingerprint.is_some()
        && report_fingerprint == context.session_fingerprint.as_deref()
        && context
            .enrolled_fingerprint
            .as_deref()
            .is_none_or(|value| Some(value) == report_fingerprint);
    if !fingerprint_bound {
        errors.push("signed report hardware fingerprint is not bound to the session".to_string());
    }
    if context.session_status == "offline" {
        warnings.push("evidence submitted while remote session is offline".to_string());
    }
    if signed.signature_valid_locally != signature_valid {
        warnings.push(
            "agent local signature flag differed from backend signature verification".to_string(),
        );
    }

    let agent_envelope_claimed_expired = signed.evidence.as_ref().map(|value| value.is_expired);
    let agent_report_claimed_expired = signed
        .report
        .evidence
        .as_ref()
        .map(|value| value.is_expired);
    if agent_envelope_claimed_expired.is_some_and(|value| value != server_freshness.is_expired) {
        warnings.push("agent envelope is_expired differed from server freshness".to_string());
    }
    if agent_report_claimed_expired.is_some_and(|value| value != server_freshness.is_expired) {
        warnings.push("agent report is_expired differed from server freshness".to_string());
    }
    if server_freshness.is_expired {
        warnings.push("signed report expired by server clock".to_string());
    }

    EvidenceVerification {
        schema_version: EVIDENCE_REGISTRY_SCHEMA_VERSION.to_string(),
        checked_at: checked_at.to_string(),
        report_hash_valid,
        evidence_hash_valid: !evidence_hash.is_empty(),
        signature_valid,
        active_key_bound,
        provider_bound,
        device_bound,
        fingerprint_bound,
        expired_by_server: server_freshness.is_expired,
        server_freshness: Some(server_freshness.clone()),
        agent_envelope_claimed_expired,
        agent_report_claimed_expired,
        warnings,
        errors,
    }
}

async fn record_rejected_evidence(
    transaction: &Transaction<'_>,
    request_id: &str,
    authorized: &AuthorizedSession,
    evidence_hash: &str,
    report_hash: &str,
    verification: &EvidenceVerification,
) -> Result<(), SessionError> {
    let metadata = serde_json::json!({
        "evidence_hash": evidence_hash,
        "report_hash": report_hash,
        "errors": &verification.errors,
        "warnings": &verification.warnings,
    })
    .to_string();
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id,
            actor_type: "device",
            actor_id: Some(authorized.device_id.clone()),
            entity_type: "provider_session",
            entity_id: &authorized.session_id,
            event_type: "evidence_record.rejected",
            idempotency_key: None,
            summary: "signed evidence rejected by remote registry",
            metadata_json: &metadata,
        },
    )
    .await?;
    Ok(())
}

async fn insert_hardware_snapshot(
    transaction: &Transaction<'_>,
    authorized: &AuthorizedSession,
    signed: &SignedReport,
    fingerprint: Option<&str>,
    observed_at: &str,
) -> Result<(), SessionError> {
    let Some(fingerprint) = fingerprint else {
        return Ok(());
    };
    let snapshot_id = format!("hardware_snapshot_{}", Uuid::new_v4());
    let payload_json = serde_json::to_string(&signed.report.system)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    transaction
        .execute(
            "INSERT INTO hardware_snapshots (snapshot_id, provider_id, device_id, hardware_fingerprint, report_hash, payload_json, observed_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &snapshot_id,
                &authorized.provider_id,
                &Some(authorized.device_id.clone()),
                &fingerprint,
                &Some(signed.report_hash.clone()),
                &payload_json,
                &observed_at,
            ],
        )
        .await?;
    Ok(())
}

async fn write_evidence_object(
    root: &str,
    provider_id: &str,
    evidence_hash: &str,
    signed: &SignedReport,
) -> Result<String, SessionError> {
    if !is_safe_identifier(provider_id, 96) || !is_hex_hash(evidence_hash) {
        return Err(SessionError::Invalid(
            "evidence object key contains unsupported characters".to_string(),
        ));
    }
    let object_key = format!("evidence/{provider_id}/{evidence_hash}.json");
    let path = object_path(root, &object_key)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create evidence object directory", parent, error))?;
    }
    if fs::metadata(&path).is_err() {
        let bytes = serde_json::to_vec_pretty(signed)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        fs::write(&path, bytes).map_err(|error| io_error("write evidence object", &path, error))?;
    }
    Ok(object_key)
}

fn object_path(root: &str, object_key: &str) -> Result<PathBuf, SessionError> {
    let root = PathBuf::from(root);
    let mut path = root;
    for component in object_key.split('/') {
        if !is_safe_identifier(component.trim_end_matches(".json"), 128) {
            return Err(SessionError::Invalid(
                "evidence object key contains unsupported characters".to_string(),
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

fn evidence_record_from_row(row: Row) -> Result<EvidenceRecord, SessionError> {
    let verification_json: String = row.get("verification_json");
    let verification: EvidenceVerification = serde_json::from_str(&verification_json)
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    Ok(EvidenceRecord {
        evidence_id: row.get("evidence_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        evidence_type: row.get("evidence_type"),
        subject_id: row.get("subject_id"),
        canonicalization_version: row.get("canonicalization_version"),
        evidence_hash: row.get("evidence_hash"),
        report_hash: row.get("report_hash"),
        hardware_fingerprint: row.get("hardware_fingerprint"),
        public_key_id: row.get("public_key_id"),
        object_key: row.get("object_key"),
        status: row.get("status"),
        server_received_at: row.get("server_received_at"),
        issued_at: row.get("issued_at"),
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
        revocation_reason: row.get("revocation_reason"),
        verification,
    })
}

fn evidence_select_sql(predicate: &str) -> &'static str {
    match predicate {
        "WHERE evidence_hash = $1" => {
            "SELECT evidence_id, provider_id, device_id, session_id, evidence_type, subject_id, canonicalization_version, evidence_hash, report_hash, hardware_fingerprint, public_key_id, object_key, status, server_received_at, issued_at, expires_at, revoked_at, revocation_reason, verification_json FROM evidence_records WHERE evidence_hash = $1"
        }
        "WHERE evidence_id = $1" => {
            "SELECT evidence_id, provider_id, device_id, session_id, evidence_type, subject_id, canonicalization_version, evidence_hash, report_hash, hardware_fingerprint, public_key_id, object_key, status, server_received_at, issued_at, expires_at, revoked_at, revocation_reason, verification_json FROM evidence_records WHERE evidence_id = $1"
        }
        "WHERE provider_id = $1 ORDER BY server_received_at DESC LIMIT $2" => {
            "SELECT evidence_id, provider_id, device_id, session_id, evidence_type, subject_id, canonicalization_version, evidence_hash, report_hash, hardware_fingerprint, public_key_id, object_key, status, server_received_at, issued_at, expires_at, revoked_at, revocation_reason, verification_json FROM evidence_records WHERE provider_id = $1 ORDER BY server_received_at DESC LIMIT $2"
        }
        _ => unreachable!("unsupported evidence select predicate"),
    }
}

fn validate_evidence_type(value: &str) -> Result<(), SessionError> {
    if is_safe_identifier(value, 64) {
        Ok(())
    } else {
        Err(SessionError::Invalid(
            "evidence_type must be a non-empty ASCII identifier".to_string(),
        ))
    }
}

fn validate_revocation_reason(value: &str) -> Result<(), SessionError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 256 || contains_secret_text(trimmed) {
        Err(SessionError::Invalid(
            "revocation reason must be non-empty, short, and free of secret material".to_string(),
        ))
    } else {
        Ok(())
    }
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

fn contains_secret_field(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .any(|(key, value)| contains_secret_text(key) || contains_secret_field(value)),
        serde_json::Value::Array(items) => items.iter().any(contains_secret_field),
        serde_json::Value::String(value) => contains_secret_text(value),
        _ => false,
    }
}

fn contains_secret_text(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "private_key",
        "secret_key",
        "secret_key_base64",
        "api_token",
        "enrollment_token",
        "credential",
        "authorization",
        "password",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{
        FullReport, ReportSignature, generate_keypair, hash_canonical, sign_message,
    };

    #[test]
    fn verification_recomputes_expiry_from_server_clock() {
        let keys = generate_keypair().unwrap();
        let report = FullReport {
            identity: None,
            evidence: Some(burd_protocol::EvidenceFreshness {
                issued_at: "2026-01-01T00:00:00+00:00".to_string(),
                expires_at: "2026-01-08T00:00:00+00:00".to_string(),
                is_expired: false,
                age_seconds: 0,
                ttl_seconds: SIGNED_REPORT_TTL_SECONDS,
            }),
            hardware_fingerprint: Some("sha256:fingerprint".to_string()),
            marketplace_policy: None,
            system: serde_json::json!({"os":"linux"}),
            fit: None,
            llm_benchmark: None,
            stability: None,
            network: None,
            network_score: None,
            disk: None,
            reliability: None,
            ai_performance: None,
            score: serde_json::json!({"burd_compute_score": 0}),
            timestamp: "2026-01-01T00:00:00+00:00".to_string(),
            agent_version: "0.1.0".to_string(),
            benchmark_version: "test".to_string(),
            benchmark_profile: "profile_8gb".to_string(),
            challenge: None,
            signature: ReportSignature {
                algorithm: KEY_ALGORITHM.to_string(),
                value: "signature-in-envelope".to_string(),
                status: "signed".to_string(),
            },
        };
        let report_hash = hash_canonical(&report).unwrap();
        let signature = sign_message(&keys.secret_key_base64, report_hash.as_bytes()).unwrap();
        let signed = SignedReport {
            provider_id: "local-provider".to_string(),
            machine_id: "machine-1".to_string(),
            report,
            report_hash: report_hash.clone(),
            signature,
            public_key: keys.public_key_base64.clone(),
            key_algorithm: KEY_ALGORITHM.to_string(),
            signed_at: "2026-01-01T00:00:00+00:00".to_string(),
            evidence: Some(burd_protocol::EvidenceFreshness {
                issued_at: "2026-01-01T00:00:00+00:00".to_string(),
                expires_at: "2026-01-08T00:00:00+00:00".to_string(),
                is_expired: false,
                age_seconds: 0,
                ttl_seconds: SIGNED_REPORT_TTL_SECONDS,
            }),
            signature_valid_locally: true,
            canonicalization_version: EVIDENCE_CANONICALIZATION_VERSION.to_string(),
        };
        let freshness = evidence_freshness_at(
            &signed.signed_at,
            SIGNED_REPORT_TTL_SECONDS,
            chrono::DateTime::parse_from_rfc3339("2026-01-09T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();
        let context = EvidenceContext {
            session_status: "online".to_string(),
            session_fingerprint: Some("sha256:fingerprint".to_string()),
            public_key_id: "key_1".to_string(),
            active_public_key: keys.public_key_base64,
            machine_id: Some("machine-1".to_string()),
            local_provider_id: Some("local-provider".to_string()),
            enrolled_fingerprint: Some("sha256:fingerprint".to_string()),
        };
        let evidence_hash = hash_canonical(&signed).unwrap();
        let verification = build_verification(
            &signed,
            &evidence_hash,
            &report_hash,
            "2026-01-09T00:00:00Z",
            &freshness,
            "provider_backend",
            &context,
        );
        assert!(verification.expired_by_server);
        assert!(verification.errors.is_empty());
        assert!(
            verification
                .warnings
                .iter()
                .any(|warning| warning.contains("is_expired differed"))
        );
    }

    #[test]
    fn object_keys_reject_path_traversal() {
        assert!(object_path("objects", "evidence/provider_1/hash.json").is_ok());
        assert!(object_path("objects", "evidence/../hash.json").is_err());
    }
}
