use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::SessionError;
use burd_protocol::{
    JOB_LEASE_SCHEMA_VERSION, JobLeaseRecord, ListJobLeasesResponse, RunSchedulerRequest,
    RunSchedulerResponse, SchedulerDecisionRecord,
};
use chrono::{Duration, Utc};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const DEFAULT_SCHEDULER_LIMIT: u32 = 50;
const MAX_SCHEDULER_LIMIT: u32 = 200;
const DEFAULT_LEASE_TTL_SECONDS: u32 = 120;
const MAX_LEASE_TTL_SECONDS: u32 = 15 * 60;
const ACTIVE_LEASE_STATUSES: &[&str] = &["offered", "accepted", "provisioning", "active"];

impl Database {
    pub async fn run_scheduler(
        &self,
        request_id: &str,
        request: &RunSchedulerRequest,
    ) -> Result<RunSchedulerResponse, SessionError> {
        validate_scheduler_request(request)?;
        let limit = request
            .limit
            .unwrap_or(DEFAULT_SCHEDULER_LIMIT)
            .clamp(1, MAX_SCHEDULER_LIMIT);
        let ttl_seconds = request
            .lease_ttl_seconds
            .unwrap_or(DEFAULT_LEASE_TTL_SECONDS)
            .min(MAX_LEASE_TTL_SECONDS);
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let expires_at = (now + Duration::seconds(i64::from(ttl_seconds))).to_rfc3339();
        let expired = expire_stale_offers(&transaction, &now_text).await?;

        let candidate_rows = transaction
            .query(
                "SELECT j.job_id, j.provider_id, j.device_id, j.session_id, j.workload_type, j.gpu_uuid, j.policy_id, j.policy_version FROM compute_jobs j JOIN providers p ON p.provider_id = j.provider_id JOIN devices d ON d.device_id = j.device_id AND d.provider_id = j.provider_id JOIN provider_sessions s ON s.session_id = j.session_id AND s.provider_id = j.provider_id AND s.device_id = j.device_id WHERE j.status = 'queued' AND p.status NOT IN ('blocked', 'quarantined') AND d.status = 'active' AND s.status IN ('online', 'degraded') AND NOT EXISTS (SELECT 1 FROM job_leases existing_job WHERE existing_job.job_id = j.job_id AND existing_job.status = ANY($2)) AND NOT EXISTS (SELECT 1 FROM job_leases active_gpu WHERE active_gpu.provider_id = j.provider_id AND active_gpu.device_id = j.device_id AND active_gpu.gpu_uuid = j.gpu_uuid AND active_gpu.status = ANY($2)) AND EXISTS (SELECT 1 FROM device_gpu_inventory inv WHERE inv.provider_id = j.provider_id AND inv.device_id = j.device_id AND inv.gpu_uuid = j.gpu_uuid AND inv.status = 'active' ORDER BY inv.server_received_at DESC LIMIT 1) AND EXISTS (SELECT 1 FROM provider_workload_eligibility e WHERE e.provider_id = j.provider_id AND e.device_id = j.device_id AND e.workload_type = j.workload_type AND e.status IN ('eligible', 'limited') AND (j.policy_id IS NULL OR e.policy_id = j.policy_id) AND (j.policy_version IS NULL OR e.policy_version = j.policy_version)) ORDER BY j.created_at ASC LIMIT $1 FOR UPDATE OF j SKIP LOCKED",
                &[&(limit as i64), &ACTIVE_LEASE_STATUSES],
            )
            .await?;

        let mut decisions = Vec::new();
        for row in candidate_rows {
            let job_id: String = row.get("job_id");
            let provider_id: String = row.get("provider_id");
            let device_id: String = row.get("device_id");
            let session_id: String = row.get("session_id");
            let workload_type: String = row.get("workload_type");
            let gpu_uuid: String = row.get("gpu_uuid");
            let policy_id: Option<String> = row.get("policy_id");
            let policy_version: Option<String> = row.get("policy_version");
            let lease_id = format!("lease_{}", Uuid::new_v4());
            let reason_codes = vec![
                "backend_eligibility_satisfied".to_string(),
                "session_online_or_degraded".to_string(),
                "gpu_not_reserved".to_string(),
            ];
            let reason_codes_json = serde_json::to_string(&reason_codes)
                .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
            transaction
                .execute(
                    "INSERT INTO job_leases (lease_id, job_id, provider_id, device_id, session_id, schema_version, workload_type, gpu_uuid, policy_id, policy_version, status, reason_codes_json, offered_at, expires_at, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'offered', $11, $12, $13, $12, $12)",
                    &[
                        &lease_id,
                        &job_id,
                        &provider_id,
                        &device_id,
                        &session_id,
                        &JOB_LEASE_SCHEMA_VERSION,
                        &workload_type,
                        &gpu_uuid,
                        &policy_id,
                        &policy_version,
                        &reason_codes_json,
                        &now_text,
                        &expires_at,
                    ],
                )
                .await?;
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "scheduler",
                    actor_id: None,
                    entity_type: "job_lease",
                    entity_id: &lease_id,
                    event_type: "lease.offered",
                    idempotency_key: None,
                    summary: "scheduler offered job lease",
                    metadata_json: "{}",
                },
            )
            .await?;
            decisions.push(SchedulerDecisionRecord {
                job_id,
                lease_id: Some(lease_id),
                provider_id,
                device_id,
                session_id,
                gpu_uuid,
                decision: "offered".to_string(),
                reason_codes,
            });
        }
        let offered = decisions.len() as u32;
        transaction.commit().await?;
        Ok(RunSchedulerResponse {
            request_id: request_id.to_string(),
            evaluated: offered,
            offered,
            expired,
            skipped: 0,
            decisions,
        })
    }

    pub async fn list_provider_job_leases(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListJobLeasesResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let limit = limit.clamp(1, MAX_SCHEDULER_LIMIT) as i64;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE provider_id = $1 ORDER BY updated_at DESC LIMIT $2",
                    lease_select_columns()
                ),
                &[&provider_id, &limit],
            )
            .await?;
        let leases = rows
            .into_iter()
            .map(lease_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListJobLeasesResponse {
            request_id: request_id.to_string(),
            leases,
        })
    }

    pub async fn list_job_leases(
        &self,
        request_id: &str,
        job_id: &str,
    ) -> Result<ListJobLeasesResponse, SessionError> {
        validate_id("job_id", job_id, 128)?;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE job_id = $1 ORDER BY updated_at DESC",
                    lease_select_columns()
                ),
                &[&job_id],
            )
            .await?;
        let leases = rows
            .into_iter()
            .map(lease_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListJobLeasesResponse {
            request_id: request_id.to_string(),
            leases,
        })
    }
}

pub(crate) async fn load_job_lease_in_transaction(
    transaction: &Transaction<'_>,
    lease_id: &str,
) -> Result<JobLeaseRecord, SessionError> {
    let row = transaction
        .query_one(
            &format!("{} WHERE lease_id = $1", lease_select_columns()),
            &[&lease_id],
        )
        .await?;
    lease_from_row(row)
}

pub(crate) async fn mark_lease_accepted_for_job(
    transaction: &Transaction<'_>,
    job_id: &str,
    now: &str,
) -> Result<(), SessionError> {
    transaction
        .execute(
            "UPDATE job_leases SET status = 'accepted', accepted_at = COALESCE(accepted_at, $1), updated_at = $1 WHERE job_id = $2 AND status = 'offered'",
            &[&now, &job_id],
        )
        .await?;
    Ok(())
}

pub(crate) async fn mark_lease_progress_for_job(
    transaction: &Transaction<'_>,
    job_id: &str,
    event_type: &str,
    now: &str,
) -> Result<(), SessionError> {
    match event_type {
        "provisioning" => {
            transaction
                .execute(
                    "UPDATE job_leases SET status = 'provisioning', provisioning_at = COALESCE(provisioning_at, $1), updated_at = $1 WHERE job_id = $2 AND status IN ('accepted', 'provisioning', 'active')",
                    &[&now, &job_id],
                )
                .await?;
        }
        "started" | "running" | "uploading" => {
            transaction
                .execute(
                    "UPDATE job_leases SET status = 'active', active_at = COALESCE(active_at, $1), updated_at = $1 WHERE job_id = $2 AND status IN ('accepted', 'provisioning', 'active')",
                    &[&now, &job_id],
                )
                .await?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) async fn mark_lease_terminal_for_job(
    transaction: &Transaction<'_>,
    job_id: &str,
    job_status: &str,
    failure_reason: Option<&str>,
    now: &str,
) -> Result<(), SessionError> {
    let lease_status = if job_status == "succeeded" {
        "completed"
    } else {
        "failed"
    };
    transaction
        .execute(
            "UPDATE job_leases SET status = $1, completed_at = $2, failure_reason = $3, updated_at = $2 WHERE job_id = $4 AND status IN ('offered', 'accepted', 'provisioning', 'active')",
            &[&lease_status, &now, &failure_reason, &job_id],
        )
        .await?;
    Ok(())
}

async fn expire_stale_offers(
    transaction: &Transaction<'_>,
    now: &str,
) -> Result<u32, SessionError> {
    let rows = transaction
        .query(
            "UPDATE job_leases SET status = 'expired', failure_reason = 'lease_ack_timeout', updated_at = $1 WHERE status = 'offered' AND expires_at <= $1 RETURNING job_id",
            &[&now],
        )
        .await?;
    for row in &rows {
        let job_id: String = row.get("job_id");
        transaction
            .execute(
                "UPDATE compute_jobs SET status = 'queued', assigned_at = NULL, job_credential_hash = NULL, job_credential_expires_at = NULL, updated_at = $1 WHERE job_id = $2 AND status = 'assigned'",
                &[&now, &job_id],
            )
            .await?;
    }
    Ok(rows.len() as u32)
}

fn lease_from_row(row: Row) -> Result<JobLeaseRecord, SessionError> {
    let reason_codes_json: String = row.get("reason_codes_json");
    Ok(JobLeaseRecord {
        lease_id: row.get("lease_id"),
        job_id: row.get("job_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        schema_version: row.get("schema_version"),
        workload_type: row.get("workload_type"),
        gpu_uuid: row.get("gpu_uuid"),
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
        status: row.get("status"),
        reason_codes: serde_json::from_str(&reason_codes_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        offered_at: row.get("offered_at"),
        expires_at: row.get("expires_at"),
        accepted_at: row.get("accepted_at"),
        provisioning_at: row.get("provisioning_at"),
        active_at: row.get("active_at"),
        completed_at: row.get("completed_at"),
        failure_reason: row.get("failure_reason"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn lease_select_columns() -> &'static str {
    "SELECT lease_id, job_id, provider_id, device_id, session_id, schema_version, workload_type, gpu_uuid, policy_id, policy_version, status, reason_codes_json, offered_at, expires_at, accepted_at, provisioning_at, active_at, completed_at, failure_reason, created_at, updated_at FROM job_leases"
}

fn validate_scheduler_request(request: &RunSchedulerRequest) -> Result<(), SessionError> {
    if let Some(limit) = request.limit
        && (limit == 0 || limit > MAX_SCHEDULER_LIMIT)
    {
        return Err(SessionError::Invalid(
            "scheduler limit is outside allowed range".to_string(),
        ));
    }
    if let Some(ttl) = request.lease_ttl_seconds
        && (ttl == 0 || ttl > MAX_LEASE_TTL_SECONDS)
    {
        return Err(SessionError::Invalid(
            "lease_ttl_seconds is outside allowed range".to_string(),
        ));
    }
    if let Some(reason) = request.reason.as_deref()
        && !is_bounded_ascii(reason, 256)
    {
        return Err(SessionError::Invalid(
            "scheduler reason must be short printable ASCII".to_string(),
        ));
    }
    Ok(())
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

fn is_bounded_ascii(value: &str, maximum_len: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum_len
        && value
            .chars()
            .all(|character| character.is_ascii() && !character.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_request_validation_rejects_unbounded_inputs() {
        assert!(validate_scheduler_request(&RunSchedulerRequest::default()).is_ok());
        assert!(
            validate_scheduler_request(&RunSchedulerRequest {
                limit: Some(0),
                lease_ttl_seconds: None,
                reason: None,
            })
            .is_err()
        );
        assert!(
            validate_scheduler_request(&RunSchedulerRequest {
                limit: None,
                lease_ttl_seconds: Some(MAX_LEASE_TTL_SECONDS + 1),
                reason: None,
            })
            .is_err()
        );
        assert!(
            validate_scheduler_request(&RunSchedulerRequest {
                limit: None,
                lease_ttl_seconds: None,
                reason: Some("token leak".to_string()),
            })
            .is_ok()
        );
    }
}
