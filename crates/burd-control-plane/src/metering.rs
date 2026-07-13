use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::SessionError;
use burd_protocol::{
    JOB_USAGE_RECEIPT_SCHEMA_VERSION, JobArtifact, JobUsageReceipt, ListUsageLedgerResponse,
    USAGE_LEDGER_SCHEMA_VERSION, UsageLedgerEntry, UsageLedgerResponse, hash_canonical,
};
use chrono::{DateTime, Utc};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const ENTRY_TYPE_JOB_USAGE_FINALIZED: &str = "job_usage_finalized";
const MAX_USAGE_LEDGER_LIMIT: u32 = 200;

#[derive(Debug, Clone)]
struct MeteredJobSource {
    job_id: String,
    provider_id: String,
    device_id: String,
    session_id: String,
    workload_type: String,
    gpu_uuid: String,
    status: String,
    input_artifacts: Vec<JobArtifact>,
    result_artifacts: Vec<JobArtifact>,
    error_code: Option<String>,
    cancellation_reason: Option<String>,
    assigned_at: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    job_updated_at: String,
    lease_id: Option<String>,
    lease_offered_at: Option<String>,
    lease_accepted_at: Option<String>,
    lease_active_at: Option<String>,
    lease_completed_at: Option<String>,
    lease_updated_at: Option<String>,
    retry_count: u32,
}

impl Database {
    pub async fn finalize_job_usage(
        &self,
        request_id: &str,
        job_id: &str,
    ) -> Result<UsageLedgerResponse, SessionError> {
        validate_id("job_id", job_id, 128)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        let (entry, duplicate) =
            append_usage_ledger_for_job(&transaction, request_id, job_id, &now).await?;
        transaction.commit().await?;
        Ok(UsageLedgerResponse {
            request_id: request_id.to_string(),
            entry,
            duplicate,
        })
    }

    pub async fn list_job_usage_ledger(
        &self,
        request_id: &str,
        job_id: &str,
    ) -> Result<ListUsageLedgerResponse, SessionError> {
        validate_id("job_id", job_id, 128)?;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE job_id = $1 ORDER BY created_at DESC",
                    usage_select_columns()
                ),
                &[&job_id],
            )
            .await?;
        let entries = rows
            .into_iter()
            .map(usage_entry_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListUsageLedgerResponse {
            request_id: request_id.to_string(),
            entries,
        })
    }

    pub async fn list_provider_usage_ledger(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListUsageLedgerResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let limit = limit.clamp(1, MAX_USAGE_LEDGER_LIMIT) as i64;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE provider_id = $1 ORDER BY created_at DESC LIMIT $2",
                    usage_select_columns()
                ),
                &[&provider_id, &limit],
            )
            .await?;
        let entries = rows
            .into_iter()
            .map(usage_entry_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListUsageLedgerResponse {
            request_id: request_id.to_string(),
            entries,
        })
    }
}

pub(crate) async fn append_usage_ledger_for_job(
    transaction: &Transaction<'_>,
    request_id: &str,
    job_id: &str,
    now: &str,
) -> Result<(UsageLedgerEntry, bool), SessionError> {
    let source = load_metered_job_source(transaction, job_id).await?;
    if !matches!(source.status.as_str(), "succeeded" | "failed" | "cancelled") {
        return Err(SessionError::Conflict(
            "usage can only be finalized for terminal jobs".to_string(),
        ));
    }
    let entry_id = format!("usage_{}", Uuid::new_v4());
    let receipt = build_job_usage_receipt(&source);
    let receipt_json = serde_json::to_string(&receipt)
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    let receipt_hash = hash_canonical(&receipt).map_err(SessionError::Invalid)?;
    let source_hash = hash_canonical(&serde_json::json!({
        "entry_type": ENTRY_TYPE_JOB_USAGE_FINALIZED,
        "job_id": source.job_id,
        "job_updated_at": source.job_updated_at,
        "lease_id": source.lease_id,
        "lease_updated_at": source.lease_updated_at,
        "retry_count": source.retry_count,
    }))
    .map_err(SessionError::Invalid)?;
    let reason_codes_json = serde_json::to_string(&receipt.reason_codes)
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    let receipt_signature_status = "hash_only_backend_signature_not_configured".to_string();

    let inserted = transaction
        .query_opt(
            "INSERT INTO usage_ledger_entries (entry_id, schema_version, entry_type, job_id, lease_id, provider_id, device_id, session_id, workload_type, gpu_uuid, job_status, lease_started_at, lease_ended_at, job_started_at, job_completed_at, reserved_gpu_seconds, actual_gpu_seconds, billable_gpu_seconds, non_billable_gpu_seconds, idle_billable_gpu_seconds, idle_unbillable_gpu_seconds, input_bytes, output_bytes, network_transfer_bytes, storage_bytes, retry_count, provider_caused_failure, customer_caused_failure, failure_classification, challenge_non_billable_seconds, reason_codes_json, receipt_json, receipt_hash, receipt_signature, receipt_public_key, receipt_signature_status, source_hash, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, NULL, NULL, $34, $35, $36) ON CONFLICT (job_id, entry_type) DO NOTHING RETURNING entry_id",
            &[
                &entry_id,
                &USAGE_LEDGER_SCHEMA_VERSION,
                &ENTRY_TYPE_JOB_USAGE_FINALIZED,
                &receipt.job_id,
                &receipt.lease_id,
                &receipt.provider_id,
                &receipt.device_id,
                &receipt.session_id,
                &receipt.workload_type,
                &receipt.gpu_uuid,
                &receipt.job_status,
                &receipt.lease_started_at,
                &receipt.lease_ended_at,
                &receipt.job_started_at,
                &receipt.job_completed_at,
                &to_i64(receipt.reserved_gpu_seconds)?,
                &to_i64(receipt.actual_gpu_seconds)?,
                &to_i64(receipt.billable_gpu_seconds)?,
                &to_i64(receipt.non_billable_gpu_seconds)?,
                &to_i64(receipt.idle_billable_gpu_seconds)?,
                &to_i64(receipt.idle_unbillable_gpu_seconds)?,
                &to_i64(receipt.input_bytes)?,
                &to_i64(receipt.output_bytes)?,
                &to_i64(receipt.network_transfer_bytes)?,
                &to_i64(receipt.storage_bytes)?,
                &(receipt.retry_count as i32),
                &receipt.provider_caused_failure,
                &receipt.customer_caused_failure,
                &receipt.failure_classification,
                &to_i64(receipt.challenge_non_billable_seconds)?,
                &reason_codes_json,
                &receipt_json,
                &receipt_hash,
                &receipt_signature_status,
                &source_hash,
                &now,
            ],
        )
        .await?;

    let duplicate = inserted.is_none();
    let row = transaction
        .query_one(
            &format!(
                "{} WHERE job_id = $1 AND entry_type = $2",
                usage_select_columns()
            ),
            &[&job_id, &ENTRY_TYPE_JOB_USAGE_FINALIZED],
        )
        .await?;
    let entry = usage_entry_from_row(row)?;
    if !duplicate {
        insert_audit_event(
            transaction,
            NewAuditEvent {
                request_id,
                actor_type: "system",
                actor_id: None,
                entity_type: "usage_ledger_entry",
                entity_id: &entry.entry_id,
                event_type: "usage.finalized",
                idempotency_key: None,
                summary: "job usage ledger entry finalized",
                metadata_json: "{}",
            },
        )
        .await?;
    }
    Ok((entry, duplicate))
}

async fn load_metered_job_source(
    transaction: &Transaction<'_>,
    job_id: &str,
) -> Result<MeteredJobSource, SessionError> {
    let row = transaction
        .query_opt(
            "SELECT j.job_id, j.provider_id, j.device_id, j.session_id, j.workload_type, j.gpu_uuid, j.status, j.input_artifacts_json, j.result_artifacts_json, j.error_code, j.cancellation_reason, j.assigned_at, j.started_at, j.completed_at, j.updated_at AS job_updated_at, l.lease_id, l.offered_at AS lease_offered_at, l.accepted_at AS lease_accepted_at, l.active_at AS lease_active_at, l.completed_at AS lease_completed_at, l.updated_at AS lease_updated_at, COALESCE((SELECT COUNT(*) FROM job_events e WHERE e.job_id = j.job_id AND e.event_type IN ('retry', 'retried', 'retrying')), 0) AS retry_count FROM compute_jobs j LEFT JOIN LATERAL (SELECT lease_id, offered_at, accepted_at, active_at, completed_at, updated_at FROM job_leases WHERE job_id = j.job_id ORDER BY created_at DESC LIMIT 1) l ON TRUE WHERE j.job_id = $1 FOR UPDATE OF j",
            &[&job_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("job not found".to_string()))?;
    let input_artifacts_json: String = row.get("input_artifacts_json");
    let result_artifacts_json: String = row.get("result_artifacts_json");
    let retry_count: i64 = row.get("retry_count");
    Ok(MeteredJobSource {
        job_id: row.get("job_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        workload_type: row.get("workload_type"),
        gpu_uuid: row.get("gpu_uuid"),
        status: row.get("status"),
        input_artifacts: serde_json::from_str(&input_artifacts_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        result_artifacts: serde_json::from_str(&result_artifacts_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        error_code: row.get("error_code"),
        cancellation_reason: row.get("cancellation_reason"),
        assigned_at: row.get("assigned_at"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        job_updated_at: row.get("job_updated_at"),
        lease_id: row.get("lease_id"),
        lease_offered_at: row.get("lease_offered_at"),
        lease_accepted_at: row.get("lease_accepted_at"),
        lease_active_at: row.get("lease_active_at"),
        lease_completed_at: row.get("lease_completed_at"),
        lease_updated_at: row.get("lease_updated_at"),
        retry_count: retry_count.max(0) as u32,
    })
}

fn build_job_usage_receipt(source: &MeteredJobSource) -> JobUsageReceipt {
    let lease_started_at = source
        .lease_accepted_at
        .clone()
        .or_else(|| source.lease_offered_at.clone())
        .or_else(|| source.assigned_at.clone());
    let lease_ended_at = source
        .lease_completed_at
        .clone()
        .or_else(|| source.completed_at.clone());
    let job_started_at = source
        .started_at
        .clone()
        .or_else(|| source.lease_active_at.clone());
    let job_completed_at = source.completed_at.clone();
    let reserved_gpu_seconds =
        seconds_between(lease_started_at.as_deref(), lease_ended_at.as_deref());
    let actual_gpu_seconds =
        seconds_between(job_started_at.as_deref(), job_completed_at.as_deref());
    let idle_gpu_seconds = reserved_gpu_seconds.saturating_sub(actual_gpu_seconds);
    let (provider_caused_failure, customer_caused_failure, failure_classification) =
        classify_failure(source);
    let billable_gpu_seconds = if source.status == "succeeded" {
        actual_gpu_seconds
    } else {
        0
    };
    let non_billable_gpu_seconds = reserved_gpu_seconds.saturating_sub(billable_gpu_seconds);
    let input_bytes = artifact_bytes(&source.input_artifacts);
    let output_bytes = artifact_bytes(&source.result_artifacts);
    let network_transfer_bytes = input_bytes.saturating_add(output_bytes);
    let storage_bytes = network_transfer_bytes;
    let mut reason_codes = vec![
        "backend_metered_usage".to_string(),
        "billing_not_executed_bn15".to_string(),
        "idle_time_unbillable_initial_policy".to_string(),
        "challenge_time_excluded".to_string(),
    ];
    if source.lease_id.is_none() {
        reason_codes.push("missing_scheduler_lease".to_string());
    }
    if failure_classification.is_some() {
        reason_codes.push("failure_classified_for_dispute_basis".to_string());
    }
    JobUsageReceipt {
        schema_version: JOB_USAGE_RECEIPT_SCHEMA_VERSION.to_string(),
        job_id: source.job_id.clone(),
        lease_id: source.lease_id.clone(),
        provider_id: source.provider_id.clone(),
        device_id: source.device_id.clone(),
        session_id: source.session_id.clone(),
        workload_type: source.workload_type.clone(),
        gpu_uuid: source.gpu_uuid.clone(),
        job_status: source.status.clone(),
        lease_started_at,
        lease_ended_at,
        job_started_at,
        job_completed_at,
        reserved_gpu_seconds,
        actual_gpu_seconds,
        billable_gpu_seconds,
        non_billable_gpu_seconds,
        idle_billable_gpu_seconds: 0,
        idle_unbillable_gpu_seconds: idle_gpu_seconds,
        input_bytes,
        output_bytes,
        network_transfer_bytes,
        storage_bytes,
        retry_count: source.retry_count,
        provider_caused_failure,
        customer_caused_failure,
        failure_classification,
        challenge_non_billable_seconds: 0,
        reason_codes,
    }
}

fn classify_failure(source: &MeteredJobSource) -> (bool, bool, Option<String>) {
    if source.status == "succeeded" {
        return (false, false, None);
    }
    if source.status == "cancelled" {
        return (false, false, Some("admin_cancelled".to_string()));
    }
    let code = source.error_code.as_deref().unwrap_or_default();
    let lower = code.to_ascii_lowercase();
    if lower.starts_with("customer_")
        || lower.starts_with("input_")
        || lower.starts_with("invalid_request")
    {
        return (false, true, Some("customer_caused_failure".to_string()));
    }
    if lower.starts_with("provider_")
        || lower.starts_with("runtime_")
        || lower.starts_with("container_")
        || lower.starts_with("gpu_")
        || lower.starts_with("infrastructure_")
        || lower.starts_with("timeout")
    {
        return (true, false, Some("provider_caused_failure".to_string()));
    }
    if source.cancellation_reason.is_some() {
        return (false, false, Some("admin_cancelled".to_string()));
    }
    (false, false, Some("unknown_failure".to_string()))
}

fn artifact_bytes(artifacts: &[JobArtifact]) -> u64 {
    artifacts
        .iter()
        .filter_map(|artifact| artifact.size_bytes)
        .fold(0_u64, u64::saturating_add)
}

fn seconds_between(start: Option<&str>, end: Option<&str>) -> u64 {
    let Some(start) = start.and_then(parse_rfc3339) else {
        return 0;
    };
    let Some(end) = end.and_then(parse_rfc3339) else {
        return 0;
    };
    let seconds = end.signed_duration_since(start).num_seconds();
    seconds.max(0) as u64
}

fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn usage_entry_from_row(row: Row) -> Result<UsageLedgerEntry, SessionError> {
    let reason_codes_json: String = row.get("reason_codes_json");
    let receipt = JobUsageReceipt {
        schema_version: JOB_USAGE_RECEIPT_SCHEMA_VERSION.to_string(),
        job_id: row.get("job_id"),
        lease_id: row.get("lease_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        workload_type: row.get("workload_type"),
        gpu_uuid: row.get("gpu_uuid"),
        job_status: row.get("job_status"),
        lease_started_at: row.get("lease_started_at"),
        lease_ended_at: row.get("lease_ended_at"),
        job_started_at: row.get("job_started_at"),
        job_completed_at: row.get("job_completed_at"),
        reserved_gpu_seconds: from_i64(row.get("reserved_gpu_seconds"))?,
        actual_gpu_seconds: from_i64(row.get("actual_gpu_seconds"))?,
        billable_gpu_seconds: from_i64(row.get("billable_gpu_seconds"))?,
        non_billable_gpu_seconds: from_i64(row.get("non_billable_gpu_seconds"))?,
        idle_billable_gpu_seconds: from_i64(row.get("idle_billable_gpu_seconds"))?,
        idle_unbillable_gpu_seconds: from_i64(row.get("idle_unbillable_gpu_seconds"))?,
        input_bytes: from_i64(row.get("input_bytes"))?,
        output_bytes: from_i64(row.get("output_bytes"))?,
        network_transfer_bytes: from_i64(row.get("network_transfer_bytes"))?,
        storage_bytes: from_i64(row.get("storage_bytes"))?,
        retry_count: from_i32(row.get("retry_count"))?,
        provider_caused_failure: row.get("provider_caused_failure"),
        customer_caused_failure: row.get("customer_caused_failure"),
        failure_classification: row.get("failure_classification"),
        challenge_non_billable_seconds: from_i64(row.get("challenge_non_billable_seconds"))?,
        reason_codes: serde_json::from_str(&reason_codes_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
    };
    Ok(UsageLedgerEntry {
        entry_id: row.get("entry_id"),
        schema_version: row.get("schema_version"),
        entry_type: row.get("entry_type"),
        receipt,
        receipt_hash: row.get("receipt_hash"),
        receipt_signature: row.get("receipt_signature"),
        receipt_public_key: row.get("receipt_public_key"),
        receipt_signature_status: row.get("receipt_signature_status"),
        source_hash: row.get("source_hash"),
        created_at: row.get("created_at"),
    })
}

fn usage_select_columns() -> &'static str {
    "SELECT entry_id, schema_version, entry_type, job_id, lease_id, provider_id, device_id, session_id, workload_type, gpu_uuid, job_status, lease_started_at, lease_ended_at, job_started_at, job_completed_at, reserved_gpu_seconds, actual_gpu_seconds, billable_gpu_seconds, non_billable_gpu_seconds, idle_billable_gpu_seconds, idle_unbillable_gpu_seconds, input_bytes, output_bytes, network_transfer_bytes, storage_bytes, retry_count, provider_caused_failure, customer_caused_failure, failure_classification, challenge_non_billable_seconds, reason_codes_json, receipt_hash, receipt_signature, receipt_public_key, receipt_signature_status, source_hash, created_at FROM usage_ledger_entries"
}

fn to_i64(value: u64) -> Result<i64, SessionError> {
    i64::try_from(value).map_err(|_| SessionError::Invalid("usage quantity overflow".to_string()))
}

fn from_i64(value: i64) -> Result<u64, SessionError> {
    u64::try_from(value)
        .map_err(|_| SessionError::Database(DbError::new("negative usage quantity")))
}

fn from_i32(value: i32) -> Result<u32, SessionError> {
    u32::try_from(value).map_err(|_| SessionError::Database(DbError::new("negative retry count")))
}

fn validate_id(label: &str, value: &str, maximum_len: usize) -> Result<(), SessionError> {
    let valid = !value.trim().is_empty()
        && value.len() <= maximum_len
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
        });
    if valid {
        Ok(())
    } else {
        Err(SessionError::Invalid(format!(
            "{label} must be a short ASCII identifier"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_receipt_calculates_seconds_bytes_and_failure_classification() {
        let source = MeteredJobSource {
            job_id: "job_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            workload_type: "llm_realtime_api".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            status: "failed".to_string(),
            input_artifacts: vec![JobArtifact {
                artifact_id: "input".to_string(),
                role: "input".to_string(),
                object_key: "jobs/job_1/input".to_string(),
                sha256: None,
                size_bytes: Some(1024),
                content_type: None,
            }],
            result_artifacts: vec![JobArtifact {
                artifact_id: "output".to_string(),
                role: "output".to_string(),
                object_key: "jobs/job_1/output".to_string(),
                sha256: None,
                size_bytes: Some(2048),
                content_type: None,
            }],
            error_code: Some("provider_runtime_error".to_string()),
            cancellation_reason: None,
            assigned_at: Some("2026-07-13T00:00:00Z".to_string()),
            started_at: Some("2026-07-13T00:02:00Z".to_string()),
            completed_at: Some("2026-07-13T00:05:00Z".to_string()),
            job_updated_at: "2026-07-13T00:05:00Z".to_string(),
            lease_id: Some("lease_1".to_string()),
            lease_offered_at: Some("2026-07-12T23:59:30Z".to_string()),
            lease_accepted_at: Some("2026-07-13T00:00:00Z".to_string()),
            lease_active_at: Some("2026-07-13T00:02:00Z".to_string()),
            lease_completed_at: Some("2026-07-13T00:05:00Z".to_string()),
            lease_updated_at: Some("2026-07-13T00:05:00Z".to_string()),
            retry_count: 2,
        };
        let receipt = build_job_usage_receipt(&source);
        assert_eq!(receipt.reserved_gpu_seconds, 300);
        assert_eq!(receipt.actual_gpu_seconds, 180);
        assert_eq!(receipt.idle_unbillable_gpu_seconds, 120);
        assert_eq!(receipt.billable_gpu_seconds, 0);
        assert_eq!(receipt.network_transfer_bytes, 3072);
        assert!(receipt.provider_caused_failure);
        assert_eq!(
            receipt.failure_classification.as_deref(),
            Some("provider_caused_failure")
        );
    }

    #[test]
    fn succeeded_usage_sets_actual_seconds_as_billable_basis() {
        let source = MeteredJobSource {
            job_id: "job_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            workload_type: "llm_realtime_api".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            status: "succeeded".to_string(),
            input_artifacts: Vec::new(),
            result_artifacts: Vec::new(),
            error_code: None,
            cancellation_reason: None,
            assigned_at: None,
            started_at: Some("2026-07-13T00:00:00Z".to_string()),
            completed_at: Some("2026-07-13T00:00:42Z".to_string()),
            job_updated_at: "2026-07-13T00:00:42Z".to_string(),
            lease_id: None,
            lease_offered_at: None,
            lease_accepted_at: Some("2026-07-13T00:00:00Z".to_string()),
            lease_active_at: None,
            lease_completed_at: Some("2026-07-13T00:00:42Z".to_string()),
            lease_updated_at: None,
            retry_count: 0,
        };
        let receipt = build_job_usage_receipt(&source);
        assert_eq!(receipt.actual_gpu_seconds, 42);
        assert_eq!(receipt.billable_gpu_seconds, 42);
        assert!(!receipt.provider_caused_failure);
        assert!(!receipt.customer_caused_failure);
    }
}
