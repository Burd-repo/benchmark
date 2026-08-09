use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::SessionError;
use crate::runtime_admission::{
    RuntimeAdmissionPolicy, evaluate_runtime_admission_for_gpu_in_transaction,
};
use burd_protocol::{
    JOB_LEASE_SCHEMA_VERSION, JobLeaseRecord, ListJobLeasesResponse, RunSchedulerRequest,
    RunSchedulerResponse, SchedulerDecisionRecord,
};
use chrono::{Duration, Utc};
use std::collections::HashSet;
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const DEFAULT_SCHEDULER_LIMIT: u32 = 50;
const MAX_SCHEDULER_LIMIT: u32 = 200;
const DEFAULT_LEASE_TTL_SECONDS: u32 = 120;
const MAX_LEASE_TTL_SECONDS: u32 = 15 * 60;
const SCHEDULER_BATCH_SIZE: u32 = 50;
const MAX_SCHEDULER_EVALUATIONS: u32 = MAX_SCHEDULER_LIMIT * 4;
const ACTIVE_LEASE_STATUSES: &[&str] = &["offered", "accepted", "provisioning", "active"];

#[derive(Debug)]
struct SchedulerCandidate {
    job_id: String,
    provider_id: String,
    device_id: String,
    session_id: String,
    workload_type: String,
    gpu_uuid: String,
    policy_id: Option<String>,
    policy_version: Option<String>,
}

impl Database {
    pub async fn run_scheduler(
        &self,
        request_id: &str,
        request: &RunSchedulerRequest,
        runtime_admission_policy: &RuntimeAdmissionPolicy,
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
        let evaluation_budget = scheduler_evaluation_budget(limit);
        let mut decisions = Vec::new();
        let mut evaluated_job_ids = Vec::new();
        let mut reserved_gpus = HashSet::new();
        let mut offered = 0_u32;
        let mut skipped = 0_u32;
        while offered < limit && (evaluated_job_ids.len() as u32) < evaluation_budget {
            let remaining_budget = evaluation_budget - evaluated_job_ids.len() as u32;
            let batch_limit = SCHEDULER_BATCH_SIZE.min(remaining_budget) as i64;
            let candidate_rows = transaction
                .query(
                    "SELECT j.job_id, j.provider_id, j.device_id, j.session_id, j.workload_type, j.gpu_uuid, j.policy_id, j.policy_version FROM compute_jobs j JOIN providers p ON p.provider_id = j.provider_id JOIN devices d ON d.device_id = j.device_id AND d.provider_id = j.provider_id JOIN provider_sessions s ON s.session_id = j.session_id AND s.provider_id = j.provider_id AND s.device_id = j.device_id WHERE j.status = 'queued' AND s.status IN ('online', 'degraded') AND NOT (j.job_id = ANY($3)) AND NOT EXISTS (SELECT 1 FROM job_leases existing_job WHERE existing_job.job_id = j.job_id AND existing_job.status = ANY($2)) AND NOT EXISTS (SELECT 1 FROM job_leases active_gpu WHERE active_gpu.provider_id = j.provider_id AND active_gpu.device_id = j.device_id AND lower(active_gpu.gpu_uuid) = lower(j.gpu_uuid) AND active_gpu.status = ANY($2)) AND EXISTS (SELECT 1 FROM provider_workload_eligibility e WHERE e.provider_id = j.provider_id AND e.device_id = j.device_id AND e.workload_type = j.workload_type AND e.status IN ('eligible', 'limited') AND (j.policy_id IS NULL OR e.policy_id = j.policy_id) AND (j.policy_version IS NULL OR e.policy_version = j.policy_version)) ORDER BY COALESCE(j.scheduler_last_evaluated_at, j.created_at) ASC, j.created_at ASC, j.job_id ASC LIMIT $1 FOR UPDATE OF j SKIP LOCKED",
                    &[&batch_limit, &ACTIVE_LEASE_STATUSES, &evaluated_job_ids],
                )
                .await?;
            if candidate_rows.is_empty() {
                break;
            }
            for row in candidate_rows {
                let candidate = scheduler_candidate_from_row(row);
                evaluated_job_ids.push(candidate.job_id.clone());
                let admission = evaluate_runtime_admission_for_gpu_in_transaction(
                    &transaction,
                    &candidate.provider_id,
                    &candidate.device_id,
                    &candidate.gpu_uuid,
                    runtime_admission_policy,
                    now,
                )
                .await?;
                if admission.status != "admitted" {
                    skipped += 1;
                    decisions.push(skipped_scheduler_decision(
                        &candidate,
                        admission.reason_codes,
                    ));
                    continue;
                }

                let gpu_lock_key = scheduler_gpu_lock_key(
                    &candidate.provider_id,
                    &candidate.device_id,
                    &candidate.gpu_uuid,
                );
                if reserved_gpus.contains(&gpu_lock_key) {
                    skipped += 1;
                    decisions.push(skipped_scheduler_decision(
                        &candidate,
                        vec!["gpu_already_reserved_in_scheduler_run".to_string()],
                    ));
                    continue;
                }
                if !try_lock_scheduler_gpu(&transaction, &gpu_lock_key).await? {
                    skipped += 1;
                    decisions.push(skipped_scheduler_decision(
                        &candidate,
                        vec!["gpu_scheduler_lock_contended".to_string()],
                    ));
                    continue;
                }
                if gpu_has_active_lease(
                    &transaction,
                    &candidate.provider_id,
                    &candidate.device_id,
                    &candidate.gpu_uuid,
                )
                .await?
                {
                    skipped += 1;
                    decisions.push(skipped_scheduler_decision(
                        &candidate,
                        vec!["gpu_already_reserved".to_string()],
                    ));
                    continue;
                }

                let lease_id = format!("lease_{}", Uuid::new_v4());
                let reason_codes = vec![
                    "backend_eligibility_satisfied".to_string(),
                    "runtime_admission_admitted".to_string(),
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
                        &candidate.job_id,
                        &candidate.provider_id,
                        &candidate.device_id,
                        &candidate.session_id,
                        &JOB_LEASE_SCHEMA_VERSION,
                        &candidate.workload_type,
                        &candidate.gpu_uuid,
                        &candidate.policy_id,
                        &candidate.policy_version,
                        &reason_codes_json,
                        &now_text,
                        &expires_at,
                    ],
                )
                .await?;
                let audit_metadata = serde_json::json!({
                    "runtime_admission": {
                        "evaluated_at": admission.evaluated_at,
                        "verification_id": admission.verification_id,
                        "runtime_verification_fingerprint": admission.runtime_verification_fingerprint,
                        "runtime_observation_hash": admission.runtime_observation_hash,
                    }
                })
                .to_string();
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
                        metadata_json: &audit_metadata,
                    },
                )
                .await?;
                reserved_gpus.insert(gpu_lock_key);
                offered += 1;
                decisions.push(SchedulerDecisionRecord {
                    job_id: candidate.job_id,
                    lease_id: Some(lease_id),
                    provider_id: candidate.provider_id,
                    device_id: candidate.device_id,
                    session_id: candidate.session_id,
                    gpu_uuid: candidate.gpu_uuid,
                    decision: "offered".to_string(),
                    reason_codes,
                });
                if offered >= limit {
                    break;
                }
            }
        }
        if !evaluated_job_ids.is_empty() {
            transaction
                .execute(
                    "UPDATE compute_jobs SET scheduler_last_evaluated_at = $1 WHERE job_id = ANY($2)",
                    &[&now_text, &evaluated_job_ids],
                )
                .await?;
        }
        let evaluated = evaluated_job_ids.len() as u32;
        transaction.commit().await?;
        Ok(RunSchedulerResponse {
            request_id: request_id.to_string(),
            evaluated,
            offered,
            expired,
            skipped,
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

fn scheduler_candidate_from_row(row: Row) -> SchedulerCandidate {
    SchedulerCandidate {
        job_id: row.get("job_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        workload_type: row.get("workload_type"),
        gpu_uuid: row.get("gpu_uuid"),
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
    }
}

fn scheduler_evaluation_budget(limit: u32) -> u32 {
    limit
        .saturating_mul(4)
        .clamp(DEFAULT_SCHEDULER_LIMIT, MAX_SCHEDULER_EVALUATIONS)
}

fn skipped_scheduler_decision(
    candidate: &SchedulerCandidate,
    reason_codes: Vec<String>,
) -> SchedulerDecisionRecord {
    SchedulerDecisionRecord {
        job_id: candidate.job_id.clone(),
        lease_id: None,
        provider_id: candidate.provider_id.clone(),
        device_id: candidate.device_id.clone(),
        session_id: candidate.session_id.clone(),
        gpu_uuid: candidate.gpu_uuid.clone(),
        decision: "skipped".to_string(),
        reason_codes,
    }
}

fn scheduler_gpu_lock_key(provider_id: &str, device_id: &str, gpu_uuid: &str) -> String {
    format!(
        "{provider_id}|{device_id}|{}",
        gpu_uuid.to_ascii_lowercase()
    )
}

async fn try_lock_scheduler_gpu(
    transaction: &Transaction<'_>,
    lock_key: &str,
) -> Result<bool, SessionError> {
    let row = transaction
        .query_one(
            "SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0)) AS acquired",
            &[&lock_key],
        )
        .await?;
    Ok(row.get("acquired"))
}

async fn gpu_has_active_lease(
    transaction: &Transaction<'_>,
    provider_id: &str,
    device_id: &str,
    gpu_uuid: &str,
) -> Result<bool, SessionError> {
    Ok(transaction
        .query_opt(
            "SELECT 1 FROM job_leases WHERE provider_id = $1 AND device_id = $2 AND lower(gpu_uuid) = lower($3) AND status = ANY($4) LIMIT 1",
            &[&provider_id, &device_id, &gpu_uuid, &ACTIVE_LEASE_STATUSES],
        )
        .await?
        .is_some())
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
pub(crate) mod tests {
    use super::*;
    use burd_protocol::{
        AGENT_RUNTIME_CONTRACT_VERSION, ProviderRuntimeObservationPayload,
        ProviderRuntimeVerificationRecord, RUNTIME_PROOF_POLICY_VERSION,
        RUNTIME_VERIFICATION_CANONICALIZATION_VERSION, RUNTIME_VERIFICATION_RECORD_SCHEMA_VERSION,
        provider_runtime_observation_hash, runtime_admission_claims_from_observation,
        runtime_admission_fingerprint,
    };

    fn proof_image() -> String {
        format!("ghcr.io/burd/runtime-proof@sha256:{}", "a".repeat(64))
    }

    pub(crate) fn runtime_policy() -> RuntimeAdmissionPolicy {
        RuntimeAdmissionPolicy {
            clock_skew_seconds: 300,
            observation_max_age_seconds: 180,
            approved_proof_image_ref: Some(proof_image()),
        }
    }

    pub(crate) async fn postgres_test_database(prefix: &str) -> Database {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("{prefix}_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();
        db
    }

    pub(crate) async fn seed_provider_and_policy(
        client: &tokio_postgres::Client,
        provider_id: &str,
        now: &str,
    ) {
        client
            .execute(
                "INSERT INTO providers (provider_id, status, created_at, updated_at) VALUES ($1, 'available', $2, $2)",
                &[&provider_id, &now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO workload_policies (policy_id, policy_version, schema_version, workload_type, display_name, requirements_json, status, created_at, updated_at) VALUES ('llm_realtime_api_cuda', '2026.07.0', 'burd-workload-policy-v1', 'llm_realtime_api', 'LLM realtime CUDA', '{}', 'active', $1, $1)",
                &[&now],
            )
            .await
            .unwrap();
    }

    pub(crate) async fn seed_device(
        client: &tokio_postgres::Client,
        provider_id: &str,
        device_id: &str,
        session_id: &str,
        now: &str,
        expires_at: &str,
    ) {
        let machine_id = format!("machine_{device_id}");
        client
            .execute(
                "INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ($1, $2, $3, 'active', $4, $4)",
                &[&device_id, &provider_id, &machine_id, &now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint) VALUES ($1, $2, $3, 'online', 0, $4, $5, $6)",
                &[&session_id, &provider_id, &device_id, &now, &expires_at, &"a".repeat(64)],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_workload_eligibility (provider_id, device_id, workload_type, policy_id, policy_version, schema_version, engine_version, status, reason_codes_json, session_status, latest_gpu_uuid, hardware_fingerprint, regional_reachability_json, evaluated_at, updated_at) VALUES ($1, $2, 'llm_realtime_api', 'llm_realtime_api_cuda', '2026.07.0', 'burd-workload-eligibility-v1', 'burd-workload-engine-v1', 'eligible', '[]', 'online', NULL, $3, '[]', $4, $4)",
                &[&provider_id, &device_id, &"a".repeat(64), &now],
            )
            .await
            .unwrap();
    }

    pub(crate) async fn seed_job(
        client: &tokio_postgres::Client,
        job_id: &str,
        provider_id: &str,
        device_id: &str,
        session_id: &str,
        gpu_uuid: &str,
        created_at: &str,
    ) {
        client
            .execute(
                "INSERT INTO compute_jobs (job_id, provider_id, device_id, session_id, schema_version, workload_type, template_id, image_ref, gpu_uuid, backend, parameters_json, input_artifacts_json, expected_outputs_json, result_artifacts_json, result_metrics_json, policy_id, policy_version, status, timeout_seconds, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 'llm_realtime_api', 'llm_inference', $6, $7, 'cuda', '{}', '[]', '[]', '[]', '{}', 'llm_realtime_api_cuda', '2026.07.0', 'queued', 300, $8, $8)",
                &[
                    &job_id,
                    &provider_id,
                    &device_id,
                    &session_id,
                    &burd_protocol::JOB_SCHEMA_VERSION,
                    &format!("ghcr.io/burd/runtime/llm@sha256:{}", "c".repeat(64)),
                    &gpu_uuid,
                    &created_at,
                ],
            )
            .await
            .unwrap();
    }

    pub(crate) async fn seed_admitted_runtime(
        client: &tokio_postgres::Client,
        provider_id: &str,
        device_id: &str,
        session_id: &str,
        gpu_uuids: &[String],
        verified_gpu_uuid: &str,
        now: chrono::DateTime<Utc>,
    ) {
        let now_text = now.to_rfc3339();
        let public_key_id = format!("key_{device_id}");
        let public_key = format!("pub_{device_id}");
        client
            .execute(
                "INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at) VALUES ($1, $2, $3, $4, 'ed25519', 'active', $5)",
                &[&public_key_id, &provider_id, &device_id, &public_key, &now_text],
            )
            .await
            .unwrap();
        let inventory_hash = format!("inventory_hash_{device_id}");
        for (index, gpu_uuid) in gpu_uuids.iter().enumerate() {
            let row_id = format!("inventory_{device_id}_{index}");
            let gpu_index = index as i32;
            client
                .execute(
                    "INSERT INTO device_gpu_inventory (inventory_row_id, provider_id, device_id, session_id, schema_version, inventory_hash, public_key_id, signature, canonicalization_version, gpu_uuid, gpu_index, backend, pci_vendor_id, pci_device_id, vram_total_mib, status, observed_at, server_received_at, payload_json, verification_json) VALUES ($1, $2, $3, $4, 'burd-device-gpu-inventory-v1', $5, $6, 'signature', 'burd-json-c14n-v1', $7, $8, 'cuda', '10de', '2684', 24576, 'active', $9, $9, '{}', '{}')",
                    &[
                        &row_id,
                        &provider_id,
                        &device_id,
                        &session_id,
                        &inventory_hash,
                        &public_key_id,
                        &gpu_uuid,
                        &gpu_index,
                        &now_text,
                    ],
                )
                .await
                .unwrap();
        }

        let observation = ProviderRuntimeObservationPayload {
            schema_version: burd_protocol::PROVIDER_RUNTIME_OBSERVATION_SCHEMA_VERSION.to_string(),
            provider_id: provider_id.to_string(),
            device_id: device_id.to_string(),
            session_id: session_id.to_string(),
            hardware_fingerprint: "a".repeat(64),
            host_os: "linux".to_string(),
            runtime_backend: "docker_linux_native".to_string(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            docker_server_version: "28.3.0".to_string(),
            nvidia_driver_version: "580.1".to_string(),
            nvidia_runtime: "nvidia".to_string(),
            gpu_uuids: gpu_uuids.to_vec(),
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            observed_at: now_text.clone(),
        };
        let observation_hash = provider_runtime_observation_hash(&observation).unwrap();
        let observation_json = serde_json::to_string(&observation).unwrap();
        let observation_id = format!("runtime_observation_{device_id}");
        client
            .execute(
                "INSERT INTO provider_runtime_observations (observation_id, observation_hash, provider_id, device_id, session_id, public_key_id, signature, canonicalization_version, hardware_fingerprint, host_os, runtime_backend, observed_at, server_received_at, payload_json) VALUES ($1, $2, $3, $4, $5, $6, 'signature', $7, $8, 'linux', 'docker_linux_native', $9, $9, $10)",
                &[
                    &observation_id,
                    &observation_hash,
                    &provider_id,
                    &device_id,
                    &session_id,
                    &public_key_id,
                    &RUNTIME_VERIFICATION_CANONICALIZATION_VERSION,
                    &observation.hardware_fingerprint,
                    &now_text,
                    &observation_json,
                ],
            )
            .await
            .unwrap();

        seed_runtime_verification_record(
            client,
            provider_id,
            device_id,
            session_id,
            verified_gpu_uuid,
            &observation,
            now,
            device_id,
        )
        .await;
    }

    pub(crate) async fn seed_additional_runtime_verification(
        client: &tokio_postgres::Client,
        provider_id: &str,
        device_id: &str,
        session_id: &str,
        gpu_uuid: &str,
        now: chrono::DateTime<Utc>,
    ) -> String {
        let row = client
            .query_one(
                "SELECT payload_json FROM provider_runtime_observations WHERE provider_id = $1 AND device_id = $2 ORDER BY server_received_at DESC LIMIT 1",
                &[&provider_id, &device_id],
            )
            .await
            .unwrap();
        let payload_json: String = row.get("payload_json");
        let observation: ProviderRuntimeObservationPayload =
            serde_json::from_str(&payload_json).unwrap();
        client
            .execute(
                "UPDATE provider_runtime_verifications SET status = 'superseded' WHERE provider_id = $1 AND device_id = $2 AND lower(gpu_uuid) = lower($3) AND status = 'verified'",
                &[&provider_id, &device_id, &gpu_uuid],
            )
            .await
            .unwrap();
        let id_suffix = format!("{device_id}_{}", Uuid::new_v4().simple());
        seed_runtime_verification_record(
            client,
            provider_id,
            device_id,
            session_id,
            gpu_uuid,
            &observation,
            now,
            &id_suffix,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_runtime_verification_record(
        client: &tokio_postgres::Client,
        provider_id: &str,
        device_id: &str,
        session_id: &str,
        gpu_uuid: &str,
        observation: &ProviderRuntimeObservationPayload,
        now: chrono::DateTime<Utc>,
        id_suffix: &str,
    ) -> String {
        let claims = runtime_admission_claims_from_observation(observation, gpu_uuid).unwrap();
        let admission_fingerprint = runtime_admission_fingerprint(&claims).unwrap();
        let challenge_id = format!("runtime_challenge_{id_suffix}");
        let verification_id = format!("runtime_verification_{id_suffix}");
        let public_key_id = format!("key_{device_id}");
        let verified_at = (now - Duration::seconds(60)).to_rfc3339();
        let verification_expires_at = (now + Duration::hours(1)).to_rfc3339();
        let record = ProviderRuntimeVerificationRecord {
            schema_version: RUNTIME_VERIFICATION_RECORD_SCHEMA_VERSION.to_string(),
            verification_id: verification_id.clone(),
            challenge_id: challenge_id.clone(),
            provider_id: provider_id.to_string(),
            device_id: device_id.to_string(),
            session_id: session_id.to_string(),
            hardware_fingerprint: observation.hardware_fingerprint.clone(),
            gpu_uuid: gpu_uuid.to_string(),
            host_os: observation.host_os.clone(),
            runtime_backend: observation.runtime_backend.clone(),
            status: "verified".to_string(),
            gpu_uuid_binding: "verified".to_string(),
            runtime_verification_fingerprint: "b".repeat(64),
            proof_policy_version: RUNTIME_PROOF_POLICY_VERSION.to_string(),
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            proof_image_digest: proof_image(),
            public_key_id: Some(public_key_id.clone()),
            runtime_admission_fingerprint: Some(admission_fingerprint.clone()),
            runtime_admission_claims: Some(claims.clone()),
            verified_at: verified_at.clone(),
            expires_at: verification_expires_at.clone(),
            reason_codes: Vec::new(),
        };
        let record_json = serde_json::to_string(&record).unwrap();
        let claims_json = serde_json::to_string(&claims).unwrap();
        let challenge_json = serde_json::json!({
            "challenge_id": challenge_id,
            "session_id": session_id,
        })
        .to_string();
        client
            .execute(
                "INSERT INTO runtime_verification_challenges (challenge_id, provider_id, device_id, session_id, gpu_uuid, runtime_backend, hardware_fingerprint, status, nonce, challenge_json, verification_ttl_seconds, issued_at, expires_at, verified_at, public_key_id) VALUES ($1, $2, $3, $4, $5, 'docker_linux_native', $6, 'verified', $7, $8, 3600, $9, $10, $9, $11)",
                &[
                    &challenge_id,
                    &provider_id,
                    &device_id,
                    &session_id,
                    &gpu_uuid,
                    &record.hardware_fingerprint,
                    &format!("nonce_{id_suffix}"),
                    &challenge_json,
                    &verified_at,
                    &verification_expires_at,
                    &public_key_id,
                ],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_runtime_verifications (verification_id, challenge_id, provider_id, device_id, session_id, gpu_uuid, runtime_backend, hardware_fingerprint, runtime_verification_fingerprint, status, verified_at, expires_at, record_json, public_key_id, runtime_admission_fingerprint, runtime_admission_claims_json) VALUES ($1, $2, $3, $4, $5, $6, 'docker_linux_native', $7, $8, 'verified', $9, $10, $11, $12, $13, $14)",
                &[
                    &verification_id,
                    &challenge_id,
                    &provider_id,
                    &device_id,
                    &session_id,
                    &gpu_uuid,
                    &record.hardware_fingerprint,
                    &record.runtime_verification_fingerprint,
                    &verified_at,
                    &verification_expires_at,
                    &record_json,
                    &public_key_id,
                    &admission_fingerprint,
                    &claims_json,
                ],
            )
            .await
            .unwrap();
        verification_id
    }

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

    #[test]
    fn scheduler_evaluation_budget_is_bounded_beyond_offer_limit() {
        assert_eq!(scheduler_evaluation_budget(1), 50);
        assert_eq!(scheduler_evaluation_budget(50), 200);
        assert_eq!(scheduler_evaluation_budget(MAX_SCHEDULER_LIMIT), 800);
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_scheduler_skips_denied_jobs_before_offering_admitted_job() {
        let db = postgres_test_database("burd_scheduler_admission").await;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let expires_at = (now + Duration::hours(2)).to_rfc3339();
        let client = db.connect().await.unwrap();
        seed_provider_and_policy(&client, "provider_1", &now_text).await;
        seed_device(
            &client,
            "provider_1",
            "device_1",
            "session_1",
            &now_text,
            &expires_at,
        )
        .await;
        let gpus = vec![
            "GPU-A".to_string(),
            "GPU-B".to_string(),
            "GPU-C".to_string(),
        ];
        for (index, gpu_uuid) in gpus.iter().enumerate() {
            seed_job(
                &client,
                &format!("job_{}", index + 1),
                "provider_1",
                "device_1",
                "session_1",
                gpu_uuid,
                &(now - Duration::seconds(30 - index as i64)).to_rfc3339(),
            )
            .await;
        }
        seed_admitted_runtime(
            &client,
            "provider_1",
            "device_1",
            "session_1",
            &gpus,
            "GPU-C",
            now,
        )
        .await;
        drop(client);

        let response = db
            .run_scheduler(
                "req_scheduler_admission",
                &RunSchedulerRequest {
                    limit: Some(1),
                    lease_ttl_seconds: Some(120),
                    reason: Some("runtime_admission_test".to_string()),
                },
                &runtime_policy(),
            )
            .await
            .unwrap();
        assert_eq!(response.evaluated, 3);
        assert_eq!(response.offered, 1);
        assert_eq!(response.skipped, 2);
        assert_eq!(response.decisions[0].job_id, "job_1");
        assert_eq!(response.decisions[0].decision, "skipped");
        assert_eq!(response.decisions[0].lease_id, None);
        assert!(
            response.decisions[0]
                .reason_codes
                .contains(&"runtime_verification_required".to_string())
        );
        assert_eq!(response.decisions[1].job_id, "job_2");
        assert_eq!(response.decisions[1].decision, "skipped");
        assert_eq!(response.decisions[2].job_id, "job_3");
        assert_eq!(response.decisions[2].decision, "offered");
        assert!(response.decisions[2].lease_id.is_some());

        let client = db.connect().await.unwrap();
        let lease_count: i64 = client
            .query_one("SELECT COUNT(*)::BIGINT AS count FROM job_leases", &[])
            .await
            .unwrap()
            .get("count");
        assert_eq!(lease_count, 1);
        let evaluated_count: i64 = client
            .query_one(
                "SELECT COUNT(*)::BIGINT AS count FROM compute_jobs WHERE scheduler_last_evaluated_at IS NOT NULL",
                &[],
            )
            .await
            .unwrap()
            .get("count");
        assert_eq!(evaluated_count, 3);
        let metadata_json: String = client
            .query_one(
                "SELECT metadata_json FROM audit_events WHERE event_type = 'lease.offered' ORDER BY occurred_at DESC LIMIT 1",
                &[],
            )
            .await
            .unwrap()
            .get("metadata_json");
        let metadata: serde_json::Value = serde_json::from_str(&metadata_json).unwrap();
        assert_eq!(
            metadata["runtime_admission"]["verification_id"],
            "runtime_verification_device_1"
        );
        assert_eq!(
            metadata["runtime_admission"]["runtime_verification_fingerprint"],
            "b".repeat(64)
        );
        drop(client);
        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_scheduler_fairness_reaches_candidate_after_denied_batch() {
        let db = postgres_test_database("burd_scheduler_fairness").await;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let expires_at = (now + Duration::hours(2)).to_rfc3339();
        let client = db.connect().await.unwrap();
        seed_provider_and_policy(&client, "provider_1", &now_text).await;
        for index in 1..=51 {
            let device_id = format!("device_{index:02}");
            let session_id = format!("session_{index:02}");
            let job_id = format!("job_{index:02}");
            let gpu_uuid = format!("GPU-{index:02}");
            seed_device(
                &client,
                "provider_1",
                &device_id,
                &session_id,
                &now_text,
                &expires_at,
            )
            .await;
            seed_job(
                &client,
                &job_id,
                "provider_1",
                &device_id,
                &session_id,
                &gpu_uuid,
                &(now - Duration::seconds(120 - index as i64)).to_rfc3339(),
            )
            .await;
            if index == 51 {
                seed_admitted_runtime(
                    &client,
                    "provider_1",
                    &device_id,
                    &session_id,
                    std::slice::from_ref(&gpu_uuid),
                    &gpu_uuid,
                    now,
                )
                .await;
            }
        }
        drop(client);

        let request = RunSchedulerRequest {
            limit: Some(1),
            lease_ttl_seconds: Some(120),
            reason: Some("fairness_test".to_string()),
        };
        let first = db
            .run_scheduler("req_scheduler_fairness_1", &request, &runtime_policy())
            .await
            .unwrap();
        assert_eq!(first.evaluated, 50);
        assert_eq!(first.offered, 0);
        assert_eq!(first.skipped, 50);
        assert!(first.decisions.iter().all(|decision| {
            decision
                .reason_codes
                .contains(&"gpu_inventory_missing".to_string())
        }));

        let second = db
            .run_scheduler("req_scheduler_fairness_2", &request, &runtime_policy())
            .await
            .unwrap();
        assert_eq!(second.evaluated, 1);
        assert_eq!(second.offered, 1);
        assert_eq!(second.skipped, 0);
        assert_eq!(second.decisions[0].job_id, "job_51");
        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_concurrent_schedulers_offer_only_one_gpu_lease() {
        let db = postgres_test_database("burd_scheduler_concurrency").await;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let expires_at = (now + Duration::hours(2)).to_rfc3339();
        let client = db.connect().await.unwrap();
        seed_provider_and_policy(&client, "provider_1", &now_text).await;
        seed_device(
            &client,
            "provider_1",
            "device_1",
            "session_1",
            &now_text,
            &expires_at,
        )
        .await;
        let gpu_uuid = "GPU-A".to_string();
        seed_admitted_runtime(
            &client,
            "provider_1",
            "device_1",
            "session_1",
            std::slice::from_ref(&gpu_uuid),
            &gpu_uuid,
            now,
        )
        .await;
        seed_job(
            &client,
            "job_1",
            "provider_1",
            "device_1",
            "session_1",
            &gpu_uuid,
            &(now - Duration::seconds(2)).to_rfc3339(),
        )
        .await;
        seed_job(
            &client,
            "job_2",
            "provider_1",
            "device_1",
            "session_1",
            &gpu_uuid,
            &(now - Duration::seconds(1)).to_rfc3339(),
        )
        .await;
        drop(client);

        let request = RunSchedulerRequest {
            limit: Some(1),
            lease_ttl_seconds: Some(120),
            reason: Some("concurrency_test".to_string()),
        };
        let policy = runtime_policy();
        let left_db = db.clone();
        let right_db = db.clone();
        let (left, right) = tokio::join!(
            left_db.run_scheduler("req_scheduler_concurrent_left", &request, &policy),
            right_db.run_scheduler("req_scheduler_concurrent_right", &request, &policy),
        );
        let left = left.unwrap();
        let right = right.unwrap();
        assert_eq!(left.offered + right.offered, 1);
        let client = db.connect().await.unwrap();
        let active_lease_count: i64 = client
            .query_one(
                "SELECT COUNT(*)::BIGINT AS count FROM job_leases WHERE status = ANY($1)",
                &[&ACTIVE_LEASE_STATUSES],
            )
            .await
            .unwrap()
            .get("count");
        assert_eq!(active_lease_count, 1);
        drop(client);
        db.drop_schema_for_test().await.unwrap();
    }
}
