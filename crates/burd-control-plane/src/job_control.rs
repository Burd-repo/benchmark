use crate::db::{Database, DbError, IdempotencyRecord, NewAuditEvent, insert_audit_event};
use crate::gpu_inventory::assert_gpu_inventory_contains;
use crate::job_artifact::validate_uploaded_job_results;
use crate::metering::append_usage_ledger_for_job;
use crate::remote_session::{AuthorizedSession, SessionError};
use crate::runtime_admission::{
    RuntimeAdmissionPolicy, evaluate_runtime_admission_for_gpu_in_transaction,
};
use crate::scheduler::{
    load_job_lease_in_transaction, mark_lease_accepted_for_job, mark_lease_progress_for_job,
    mark_lease_terminal_for_job,
};
use burd_protocol::{
    AcceptJobRequest, CancelJobRequest, CreateJobRequest, CreateJobResponse,
    JOB_DATA_PLANE_GRANT_VERSION, JOB_EVENT_SCHEMA_VERSION, JOB_EXECUTION_CONTROL_SCHEMA_VERSION,
    JOB_SCHEMA_VERSION, JobArtifact, JobDataPlaneGrant, JobDataPlaneUrl, JobEventRecord,
    JobEventRequest, JobEventResponse, JobExecutionControlResponse, JobExecutionDirective,
    JobLeaseRecord, JobRecord, JobResponse, ListJobsResponse, NextJobResponse,
    PROVIDER_JOB_APPROVED_TEMPLATES, PROVIDER_JOB_EXECUTION_POLICY_VERSION,
    PROVIDER_JOB_EXECUTION_SCHEMA_VERSION, ProviderJobCancellationPolicy, ProviderJobCleanupPolicy,
    ProviderJobExecutionSpec, ProviderJobExecutionState, ProviderJobRuntimePolicy,
    RuntimeAdmissionDecision, SubmitJobResultRequest, SubmitJobResultResponse, random_token,
    sha256_hex, validate_provider_job_execution_bundle,
};
use chrono::{Duration, Utc};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const DEFAULT_JOB_TIMEOUT_SECONDS: u32 = 3600;
const MAX_JOB_TIMEOUT_SECONDS: u32 = 24 * 60 * 60;
const MAX_JOB_ARTIFACTS: usize = 32;
const MAX_JOB_MESSAGE_LEN: usize = 512;
const MAX_ASSIGNMENT_OFFER_SCAN: i64 = 16;
#[derive(Debug, Clone)]
pub struct CreateJobCommand {
    pub request_id: String,
    pub scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub request: CreateJobRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateJobOutcome {
    Response(IdempotencyRecord),
    Conflict,
}

#[derive(Debug, Clone)]
struct JobEligibility {
    policy_id: Option<String>,
    policy_version: Option<String>,
    status: String,
}

impl Database {
    pub async fn create_job_idempotently(
        &self,
        command: CreateJobCommand,
    ) -> Result<CreateJobOutcome, SessionError> {
        validate_create_job_request(&command.request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        let reserved = transaction
            .execute(
                "INSERT INTO idempotency_keys (scope, idempotency_key, request_hash, status_code, response_json, created_at) VALUES ($1, $2, $3, 0, '', $4) ON CONFLICT (scope, idempotency_key) DO NOTHING",
                &[&command.scope, &command.idempotency_key, &command.request_hash, &now],
            )
            .await?
            == 1;
        if !reserved {
            let row = transaction
                .query_one(
                    "SELECT request_hash, status_code, response_json FROM idempotency_keys WHERE scope = $1 AND idempotency_key = $2 FOR UPDATE",
                    &[&command.scope, &command.idempotency_key],
                )
                .await?;
            let record = idempotency_from_row(row);
            transaction.commit().await?;
            return if record.request_hash == command.request_hash {
                Ok(CreateJobOutcome::Response(record))
            } else {
                Ok(CreateJobOutcome::Conflict)
            };
        }

        assert_job_target_is_authorized(&transaction, &command.request).await?;
        let eligibility = load_job_eligibility(&transaction, &command.request).await?;
        if !matches!(eligibility.status.as_str(), "eligible" | "limited") {
            return Err(SessionError::Conflict(
                "job target is not eligible for this workload".to_string(),
            ));
        }
        assert_gpu_inventory_contains(
            &transaction,
            &command.request.provider_id,
            &command.request.device_id,
            &command.request.gpu_uuid,
        )
        .await?;

        if let Some(client_job_id) = command.request.client_job_id.as_deref()
            && transaction
                .query_opt(
                    "SELECT job_id FROM compute_jobs WHERE provider_id = $1 AND client_job_id = $2 FOR UPDATE",
                    &[&command.request.provider_id, &client_job_id],
                )
                .await?
                .is_some()
        {
            return Err(SessionError::Conflict(
                "client_job_id already exists for provider".to_string(),
            ));
        }
        let job_id = format!("job_{}", Uuid::new_v4());
        let timeout_seconds = command
            .request
            .timeout_seconds
            .unwrap_or(DEFAULT_JOB_TIMEOUT_SECONDS);
        let parameters_json =
            normalized_json_object(&command.request.parameters, "job parameters")?;
        let input_artifacts_json = artifacts_json(&command.request.input_artifacts)?;
        let expected_outputs_json = artifacts_json(&command.request.expected_outputs)?;
        transaction
            .execute(
                "INSERT INTO compute_jobs (job_id, client_job_id, provider_id, device_id, session_id, schema_version, workload_type, template_id, image_ref, gpu_uuid, backend, parameters_json, input_artifacts_json, expected_outputs_json, result_artifacts_json, result_metrics_json, policy_id, policy_version, status, timeout_seconds, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, '[]', '{}', $15, $16, 'queued', $17, $18, $18)",
                &[
                    &job_id,
                    &command.request.client_job_id,
                    &command.request.provider_id,
                    &command.request.device_id,
                    &command.request.session_id,
                    &JOB_SCHEMA_VERSION,
                    &command.request.workload_type,
                    &command.request.template_id,
                    &command.request.image_ref,
                    &command.request.gpu_uuid,
                    &command.request.backend,
                    &parameters_json,
                    &input_artifacts_json,
                    &expected_outputs_json,
                    &eligibility.policy_id,
                    &eligibility.policy_version,
                    &(timeout_seconds as i32),
                    &now,
                ],
            )
            .await?;
        let job = load_job_in_transaction(&transaction, &job_id).await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id: &command.request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "compute_job",
                entity_id: &job.job_id,
                event_type: "job.created",
                idempotency_key: Some(command.idempotency_key.clone()),
                summary: "compute job queued for provider device",
                metadata_json: "{}",
            },
        )
        .await?;
        let response_json = serde_json::to_string(&CreateJobResponse {
            request_id: command.request_id,
            job,
            duplicate: false,
        })
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let status_code = 201_i32;
        transaction
            .execute(
                "UPDATE idempotency_keys SET status_code = $1, response_json = $2 WHERE scope = $3 AND idempotency_key = $4",
                &[&status_code, &response_json, &command.scope, &command.idempotency_key],
            )
            .await?;
        transaction.commit().await?;
        Ok(CreateJobOutcome::Response(IdempotencyRecord {
            request_hash: command.request_hash,
            status_code: status_code as u16,
            response_json,
        }))
    }

    pub async fn get_job(
        &self,
        request_id: &str,
        job_id: &str,
    ) -> Result<JobResponse, SessionError> {
        validate_id("job_id", job_id, 128)?;
        let client = self.connect().await?;
        let row = client
            .query_opt(
                &format!("{} WHERE job_id = $1", job_select_columns()),
                &[&job_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("job not found".to_string()))?;
        Ok(JobResponse {
            request_id: request_id.to_string(),
            job: job_from_row(row)?,
        })
    }

    pub async fn list_provider_jobs(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListJobsResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let limit = limit.clamp(1, 200) as i64;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE provider_id = $1 ORDER BY created_at DESC LIMIT $2",
                    job_select_columns()
                ),
                &[&provider_id, &limit],
            )
            .await?;
        let jobs = rows
            .into_iter()
            .map(job_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListJobsResponse {
            request_id: request_id.to_string(),
            jobs,
        })
    }
    pub async fn next_job_for_session(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        runtime_admission_policy: &RuntimeAdmissionPolicy,
    ) -> Result<NextJobResponse, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let lease_rows = transaction
            .query(
                "SELECT l.lease_id, l.job_id, l.gpu_uuid FROM job_leases l JOIN compute_jobs j ON j.job_id = l.job_id WHERE l.provider_id = $1 AND l.device_id = $2 AND l.session_id = $3 AND l.status = 'offered' AND l.expires_at > $4 AND j.status = 'queued' ORDER BY l.offered_at ASC, l.lease_id ASC FOR UPDATE OF l, j SKIP LOCKED LIMIT $5",
                &[
                    &authorized.provider_id,
                    &authorized.device_id,
                    &authorized.session_id,
                    &now_text,
                    &MAX_ASSIGNMENT_OFFER_SCAN,
                ],
            )
            .await?;
        for lease_row in lease_rows {
            let lease_id: String = lease_row.get("lease_id");
            let job_id: String = lease_row.get("job_id");
            let lease_gpu_uuid: String = lease_row.get("gpu_uuid");
            let queued = locked_job(&transaction, &job_id).await?;
            if queued.status != "queued" {
                return Err(SessionError::Conflict(
                    "leased job is no longer queued".to_string(),
                ));
            }
            if queued.provider_id != authorized.provider_id
                || queued.device_id != authorized.device_id
                || queued.session_id != authorized.session_id
                || !queued.gpu_uuid.eq_ignore_ascii_case(&lease_gpu_uuid)
            {
                withhold_assignment(
                    &transaction,
                    request_id,
                    authorized,
                    &lease_id,
                    &queued.job_id,
                    "lease_job_binding_mismatch",
                    &["lease_job_binding_mismatch".to_string()],
                    None,
                    &now_text,
                )
                .await?;
                continue;
            }
            let admission = evaluate_runtime_admission_for_gpu_in_transaction(
                &transaction,
                &authorized.provider_id,
                &authorized.device_id,
                &lease_gpu_uuid,
                runtime_admission_policy,
                now,
            )
            .await?;
            if admission.status != "admitted" {
                withhold_assignment(
                    &transaction,
                    request_id,
                    authorized,
                    &lease_id,
                    &queued.job_id,
                    "runtime_admission_lost_before_assignment",
                    &admission.reason_codes,
                    Some(&admission),
                    &now_text,
                )
                .await?;
                continue;
            }

            let credential = random_token("jobcred").map_err(SessionError::Invalid)?;
            let credential_hash = sha256_hex(credential.as_bytes());
            let credential_expires_at =
                (now + Duration::seconds(i64::from(queued.timeout_seconds) + 600)).to_rfc3339();
            let updated = transaction
                .execute(
                    "UPDATE compute_jobs SET status = 'assigned', assigned_at = $1, assignment_lease_id = $2, job_credential_hash = $3, job_credential_expires_at = $4, updated_at = $1 WHERE job_id = $5 AND status = 'queued' AND assignment_lease_id IS NULL",
                    &[
                        &now_text,
                        &lease_id,
                        &credential_hash,
                        &credential_expires_at,
                        &queued.job_id,
                    ],
                )
                .await?;
            if updated != 1 {
                return Err(SessionError::Conflict(
                    "leased job is no longer queued".to_string(),
                ));
            }
            let job = load_job_in_transaction(&transaction, &queued.job_id).await?;
            let lease = load_job_lease_in_transaction(&transaction, &lease_id).await?;
            let data_plane = data_plane_grant(&job, credential, credential_expires_at);
            let execution = provider_job_execution_spec(&job, &lease, &data_plane)?;
            let audit_metadata = serde_json::json!({
                "lease_id": lease_id,
                "runtime_admission": runtime_admission_audit_metadata(&admission),
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "device",
                    actor_id: Some(authorized.device_id.clone()),
                    entity_type: "compute_job",
                    entity_id: &job.job_id,
                    event_type: "job.assigned",
                    idempotency_key: None,
                    summary: "compute job assigned to provider session",
                    metadata_json: &audit_metadata,
                },
            )
            .await?;
            transaction.commit().await?;
            return Ok(NextJobResponse {
                request_id: request_id.to_string(),
                job: Some(job),
                data_plane: Some(data_plane),
                lease: Some(lease),
                execution: Some(execution),
            });
        }
        transaction.commit().await?;
        Ok(empty_next_job_response(request_id))
    }
    pub async fn accept_job(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        job_id: &str,
        request: &AcceptJobRequest,
        runtime_admission_policy: &RuntimeAdmissionPolicy,
    ) -> Result<JobResponse, SessionError> {
        validate_id("lease_id", &request.lease_id, 128)?;
        validate_job_message(request.status_message.as_deref())?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let (job, lease) =
            locked_authorized_assignment(&transaction, authorized, job_id, &request.lease_id)
                .await?;
        if job.status != "assigned" {
            return Err(SessionError::Conflict(
                "job must be assigned before it can be accepted".to_string(),
            ));
        }
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        if let Some(failure_reason) = acceptance_authority_failure(&job, &lease, authorized, &now) {
            let reason_codes = vec![failure_reason.to_string()];
            withhold_acceptance(
                &transaction,
                AcceptanceWithholding {
                    request_id,
                    authorized,
                    job: &job,
                    lease: &lease,
                    failure_reason,
                    reason_codes: &reason_codes,
                    admission: None,
                    now: &now_text,
                },
            )
            .await?;
            transaction.commit().await?;
            return Err(SessionError::Conflict(
                "job acceptance authority is no longer valid".to_string(),
            ));
        }
        let admission = evaluate_runtime_admission_for_gpu_in_transaction(
            &transaction,
            &authorized.provider_id,
            &authorized.device_id,
            &lease.gpu_uuid,
            runtime_admission_policy,
            now,
        )
        .await?;
        if admission.status != "admitted" {
            withhold_acceptance(
                &transaction,
                AcceptanceWithholding {
                    request_id,
                    authorized,
                    job: &job,
                    lease: &lease,
                    failure_reason: "runtime_admission_lost_before_acceptance",
                    reason_codes: &admission.reason_codes,
                    admission: Some(&admission),
                    now: &now_text,
                },
            )
            .await?;
            transaction.commit().await?;
            return Err(SessionError::Conflict(
                "job acceptance authority is no longer valid".to_string(),
            ));
        }
        let updated_job = transaction
            .execute(
                "UPDATE compute_jobs SET status = 'accepted', accepted_at = $1, status_message = $2, updated_at = $1 WHERE job_id = $3 AND status = 'assigned'",
                &[&now_text, &request.status_message, &job.job_id],
            )
            .await?;
        if updated_job != 1 {
            return Err(SessionError::Conflict(
                "assigned job changed before it could be accepted".to_string(),
            ));
        }
        mark_lease_accepted_for_job(&transaction, &lease.lease_id, &job.job_id, &now_text).await?;
        let updated = load_job_in_transaction(&transaction, &job.job_id).await?;
        let audit_metadata = serde_json::json!({
            "lease_id": lease.lease_id,
            "runtime_admission": runtime_admission_audit_metadata(&admission),
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device",
                actor_id: Some(authorized.device_id.clone()),
                entity_type: "compute_job",
                entity_id: &job.job_id,
                event_type: "job.accepted",
                idempotency_key: None,
                summary: "compute job accepted by provider",
                metadata_json: &audit_metadata,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(JobResponse {
            request_id: request_id.to_string(),
            job: updated,
        })
    }

    pub async fn job_execution_control(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        job_id: &str,
        lease_id: &str,
    ) -> Result<JobExecutionControlResponse, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let (job, lease) =
            locked_authorized_assignment(&transaction, authorized, job_id, lease_id).await?;
        let (directive, reason_code) = match job.status.as_str() {
            "accepted" | "provisioning" | "running" | "uploading"
                if execution_lease_matches_job_state(&job.status, &lease.status) =>
            {
                (JobExecutionDirective::Continue, None)
            }
            "cancelled" => (
                JobExecutionDirective::Cancel,
                Some("job_cancelled".to_string()),
            ),
            "succeeded" | "failed" => (
                JobExecutionDirective::Cancel,
                Some("job_terminal".to_string()),
            ),
            _ => {
                return Err(SessionError::Conflict(
                    "job execution authority is no longer active".to_string(),
                ));
            }
        };
        let response = JobExecutionControlResponse {
            schema_version: JOB_EXECUTION_CONTROL_SCHEMA_VERSION.to_string(),
            request_id: request_id.to_string(),
            job_id: job.job_id,
            lease_id: lease.lease_id,
            directive,
            reason_code,
            server_time: Utc::now().to_rfc3339(),
        };
        transaction.commit().await?;
        Ok(response)
    }

    pub async fn record_job_event(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        job_id: &str,
        request: &JobEventRequest,
    ) -> Result<JobEventResponse, SessionError> {
        validate_job_event_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let job = locked_authorized_job(&transaction, authorized, job_id).await?;
        if is_terminal_status(&job.status) {
            return Err(SessionError::Conflict(
                "terminal jobs cannot accept new events".to_string(),
            ));
        }
        if !matches!(
            job.status.as_str(),
            "accepted" | "provisioning" | "running" | "uploading"
        ) {
            return Err(SessionError::Conflict(
                "job must be accepted before provider events can be recorded".to_string(),
            ));
        }
        if transaction
            .query_opt(
                "SELECT event_id FROM job_events WHERE job_id = $1 AND sequence = $2",
                &[&job.job_id, &(request.sequence as i64)],
            )
            .await?
            .is_some()
        {
            return Err(SessionError::Conflict(
                "job event sequence already exists".to_string(),
            ));
        }
        let metadata_json = normalized_json_object(&request.metadata, "job event metadata")?;
        let now = Utc::now().to_rfc3339();
        let occurred_at = request.occurred_at.clone().unwrap_or_else(|| now.clone());
        let event_id = format!("job_event_{}", Uuid::new_v4());
        transaction
            .execute(
                "INSERT INTO job_events (event_id, job_id, provider_id, device_id, session_id, sequence, schema_version, event_type, progress_percent, message, metadata_json, occurred_at, server_received_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                &[
                    &event_id,
                    &job.job_id,
                    &job.provider_id,
                    &job.device_id,
                    &job.session_id,
                    &(request.sequence as i64),
                    &JOB_EVENT_SCHEMA_VERSION,
                    &request.event_type,
                    &request.progress_percent,
                    &request.message,
                    &metadata_json,
                    &occurred_at,
                    &now,
                ],
            )
            .await?;
        apply_event_state_update(&transaction, &job, request, &now).await?;
        mark_lease_progress_for_job(&transaction, &job.job_id, &request.event_type, &now).await?;
        let event = load_event_in_transaction(&transaction, &event_id).await?;
        let updated = load_job_in_transaction(&transaction, &job.job_id).await?;
        transaction.commit().await?;
        Ok(JobEventResponse {
            request_id: request_id.to_string(),
            event,
            job: updated,
        })
    }
    pub async fn submit_job_result(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        job_id: &str,
        request: &SubmitJobResultRequest,
    ) -> Result<SubmitJobResultResponse, SessionError> {
        validate_job_result_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let job = locked_authorized_job(&transaction, authorized, job_id).await?;
        if is_terminal_status(&job.status) {
            return Err(SessionError::Conflict(
                "terminal job result cannot be changed".to_string(),
            ));
        }
        if !matches!(
            job.status.as_str(),
            "accepted" | "provisioning" | "running" | "uploading"
        ) {
            return Err(SessionError::Conflict(
                "job must be accepted before a result can be submitted".to_string(),
            ));
        }
        validate_uploaded_job_results(
            &transaction,
            &job.job_id,
            &job.expected_outputs,
            &request.status,
            &request.result_artifacts,
        )
        .await?;
        let result_artifacts_json = artifacts_json(&request.result_artifacts)?;
        let result_metrics_json = normalized_json_object(&request.metrics, "job result metrics")?;
        let completed_at = Utc::now().to_rfc3339();
        transaction
            .execute(
                "UPDATE compute_jobs SET status = $1, result_artifacts_json = $2, result_metrics_json = $3, error_code = $4, error_message = $5, completed_at = $6, updated_at = $6, job_credential_hash = NULL, job_credential_expires_at = NULL WHERE job_id = $7",
                &[
                    &request.status,
                    &result_artifacts_json,
                    &result_metrics_json,
                    &request.error_code,
                    &request.error_message,
                    &completed_at,
                    &job.job_id,
                ],
            )
            .await?;
        release_customer_placement(&transaction, &job.job_id).await?;
        let updated = load_job_in_transaction(&transaction, &job.job_id).await?;
        mark_lease_terminal_for_job(
            &transaction,
            &job.job_id,
            &request.status,
            request.error_message.as_deref(),
            &completed_at,
        )
        .await?;
        append_usage_ledger_for_job(&transaction, request_id, &job.job_id, &completed_at).await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device",
                actor_id: Some(authorized.device_id.clone()),
                entity_type: "compute_job",
                entity_id: &job.job_id,
                event_type: if request.status == "succeeded" {
                    "job.succeeded"
                } else {
                    "job.failed"
                },
                idempotency_key: None,
                summary: "compute job result submitted",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(SubmitJobResultResponse {
            request_id: request_id.to_string(),
            job: updated,
        })
    }

    pub async fn cancel_job(
        &self,
        request_id: &str,
        job_id: &str,
        request: &CancelJobRequest,
    ) -> Result<JobResponse, SessionError> {
        validate_id("job_id", job_id, 128)?;
        validate_job_message(request.reason.as_deref())?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let job = locked_job(&transaction, job_id).await?;
        if is_terminal_status(&job.status) {
            return Err(SessionError::Conflict(
                "terminal jobs cannot be cancelled".to_string(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        transaction
            .execute(
                "UPDATE compute_jobs SET status = 'cancelled', cancellation_reason = $1, completed_at = $2, updated_at = $2, job_credential_hash = NULL, job_credential_expires_at = NULL WHERE job_id = $3",
                &[&request.reason, &now, &job.job_id],
            )
            .await?;
        release_customer_placement(&transaction, &job.job_id).await?;
        let updated = load_job_in_transaction(&transaction, &job.job_id).await?;
        mark_lease_terminal_for_job(
            &transaction,
            &job.job_id,
            "cancelled",
            request.reason.as_deref().or(Some("job_cancelled")),
            &now,
        )
        .await?;
        append_usage_ledger_for_job(&transaction, request_id, &job.job_id, &now).await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "compute_job",
                entity_id: &job.job_id,
                event_type: "job.cancelled",
                idempotency_key: None,
                summary: "compute job cancelled",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(JobResponse {
            request_id: request_id.to_string(),
            job: updated,
        })
    }
}

async fn assert_job_target_is_authorized(
    transaction: &Transaction<'_>,
    request: &CreateJobRequest,
) -> Result<(), SessionError> {
    let row = transaction
        .query_opt(
            "SELECT p.status AS provider_status, d.status AS device_status, s.status AS session_status FROM providers p JOIN devices d ON d.provider_id = p.provider_id JOIN provider_sessions s ON s.provider_id = p.provider_id AND s.device_id = d.device_id WHERE p.provider_id = $1 AND d.device_id = $2 AND s.session_id = $3",
            &[&request.provider_id, &request.device_id, &request.session_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("provider, device, or session not found".to_string()))?;
    let provider_status: String = row.get("provider_status");
    let device_status: String = row.get("device_status");
    let session_status: String = row.get("session_status");
    if matches!(provider_status.as_str(), "blocked" | "quarantined") || device_status != "active" {
        return Err(SessionError::Conflict(
            "job target provider or device is blocked".to_string(),
        ));
    }
    if !matches!(session_status.as_str(), "online" | "degraded") {
        return Err(SessionError::Conflict(
            "job target requires an online or degraded remote session".to_string(),
        ));
    }
    Ok(())
}

async fn load_job_eligibility(
    transaction: &Transaction<'_>,
    request: &CreateJobRequest,
) -> Result<JobEligibility, SessionError> {
    let row = transaction
        .query_opt(
            "SELECT policy_id, policy_version, status FROM provider_workload_eligibility WHERE provider_id = $1 AND device_id = $2 AND workload_type = $3 AND ($4::TEXT IS NULL OR policy_id = $4) AND ($5::TEXT IS NULL OR policy_version = $5) ORDER BY evaluated_at DESC LIMIT 1",
            &[
                &request.provider_id,
                &request.device_id,
                &request.workload_type,
                &request.policy_id,
                &request.policy_version,
            ],
        )
        .await?
        .ok_or_else(|| {
            SessionError::Conflict("job target has no backend workload eligibility".to_string())
        })?;
    Ok(JobEligibility {
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
        status: row.get("status"),
    })
}

async fn locked_authorized_job(
    transaction: &Transaction<'_>,
    authorized: &AuthorizedSession,
    job_id: &str,
) -> Result<JobRecord, SessionError> {
    validate_id("job_id", job_id, 128)?;
    let job = locked_job(transaction, job_id).await?;
    if job.provider_id != authorized.provider_id
        || job.device_id != authorized.device_id
        || job.session_id != authorized.session_id
    {
        return Err(SessionError::Unauthorized);
    }
    Ok(job)
}

async fn locked_authorized_assignment(
    transaction: &Transaction<'_>,
    authorized: &AuthorizedSession,
    job_id: &str,
    lease_id: &str,
) -> Result<(JobRecord, JobLeaseRecord), SessionError> {
    validate_id("job_id", job_id, 128)?;
    validate_id("lease_id", lease_id, 128)?;
    let lease_row = transaction
        .query_opt(
            "SELECT l.lease_id, j.assignment_lease_id FROM compute_jobs j JOIN job_leases l ON l.job_id = j.job_id WHERE j.job_id = $1 AND l.lease_id = $2 AND j.provider_id = $3 AND j.device_id = $4 AND j.session_id = $5 AND l.provider_id = $3 AND l.device_id = $4 AND l.session_id = $5 FOR UPDATE OF j, l",
            &[
                &job_id,
                &lease_id,
                &authorized.provider_id,
                &authorized.device_id,
                &authorized.session_id,
            ],
        )
        .await?;
    let Some(lease_row) = lease_row else {
        locked_authorized_job(transaction, authorized, job_id).await?;
        return Err(SessionError::Conflict(
            "stale or mismatched assignment acknowledgement".to_string(),
        ));
    };
    let assignment_lease_id: Option<String> = lease_row.get("assignment_lease_id");
    if assignment_lease_id.as_deref() != Some(lease_id) {
        return Err(SessionError::Conflict(
            "stale assignment acknowledgement".to_string(),
        ));
    }
    let job = load_job_in_transaction(transaction, job_id).await?;
    let lease = load_job_lease_in_transaction(transaction, lease_id).await?;
    Ok((job, lease))
}

fn acceptance_authority_failure(
    job: &JobRecord,
    lease: &JobLeaseRecord,
    authorized: &AuthorizedSession,
    now: &chrono::DateTime<Utc>,
) -> Option<&'static str> {
    if lease.status != "offered" {
        return Some(if lease.status == "expired" {
            "lease_expired_before_acceptance"
        } else {
            "lease_not_offered_before_acceptance"
        });
    }
    let Ok(expires_at) = chrono::DateTime::parse_from_rfc3339(&lease.expires_at) else {
        return Some("lease_expiry_invalid_before_acceptance");
    };
    if expires_at.with_timezone(&Utc) <= *now {
        return Some("lease_expired_before_acceptance");
    }
    if lease.job_id != job.job_id
        || lease.provider_id != authorized.provider_id
        || lease.device_id != authorized.device_id
        || lease.session_id != authorized.session_id
        || lease.provider_id != job.provider_id
        || lease.device_id != job.device_id
        || lease.session_id != job.session_id
        || !lease.gpu_uuid.eq_ignore_ascii_case(&job.gpu_uuid)
    {
        return Some("lease_job_binding_mismatch_before_acceptance");
    }
    None
}

fn execution_lease_matches_job_state(job_status: &str, lease_status: &str) -> bool {
    matches!(
        (job_status, lease_status),
        ("accepted", "accepted")
            | ("provisioning", "provisioning")
            | ("running" | "uploading", "active")
    )
}

async fn locked_job(
    transaction: &Transaction<'_>,
    job_id: &str,
) -> Result<JobRecord, SessionError> {
    let row = transaction
        .query_opt(
            &format!("{} WHERE job_id = $1 FOR UPDATE", job_select_columns()),
            &[&job_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("job not found".to_string()))?;
    job_from_row(row)
}

async fn load_job_in_transaction(
    transaction: &Transaction<'_>,
    job_id: &str,
) -> Result<JobRecord, SessionError> {
    let row = transaction
        .query_one(
            &format!("{} WHERE job_id = $1", job_select_columns()),
            &[&job_id],
        )
        .await?;
    job_from_row(row)
}

async fn load_event_in_transaction(
    transaction: &Transaction<'_>,
    event_id: &str,
) -> Result<JobEventRecord, SessionError> {
    let row = transaction
        .query_one(
            "SELECT event_id, job_id, provider_id, device_id, session_id, sequence, schema_version, event_type, progress_percent, message, metadata_json, occurred_at, server_received_at FROM job_events WHERE event_id = $1",
            &[&event_id],
        )
        .await?;
    event_from_row(row)
}
async fn apply_event_state_update(
    transaction: &Transaction<'_>,
    job: &JobRecord,
    request: &JobEventRequest,
    now: &str,
) -> Result<(), SessionError> {
    let next_status = match request.event_type.as_str() {
        "provisioning" => Some("provisioning"),
        "started" | "running" => Some("running"),
        "uploading" => Some("uploading"),
        _ => None,
    };
    if let Some(status) = next_status {
        if status == "running" {
            transaction
                .execute(
                    "UPDATE compute_jobs SET status = $1, progress_percent = $2, status_message = $3, started_at = COALESCE(started_at, $4), updated_at = $4 WHERE job_id = $5",
                    &[&status, &request.progress_percent, &request.message, &now, &job.job_id],
                )
                .await?;
        } else {
            transaction
                .execute(
                    "UPDATE compute_jobs SET status = $1, progress_percent = $2, status_message = $3, updated_at = $4 WHERE job_id = $5",
                    &[&status, &request.progress_percent, &request.message, &now, &job.job_id],
                )
                .await?;
        }
    } else {
        transaction
            .execute(
                "UPDATE compute_jobs SET progress_percent = COALESCE($1, progress_percent), status_message = COALESCE($2, status_message), updated_at = $3 WHERE job_id = $4",
                &[&request.progress_percent, &request.message, &now, &job.job_id],
            )
            .await?;
    }
    Ok(())
}

fn empty_next_job_response(request_id: &str) -> NextJobResponse {
    NextJobResponse {
        request_id: request_id.to_string(),
        job: None,
        data_plane: None,
        lease: None,
        execution: None,
    }
}

fn runtime_admission_audit_metadata(admission: &RuntimeAdmissionDecision) -> serde_json::Value {
    serde_json::json!({
        "status": admission.status,
        "reason_codes": admission.reason_codes,
        "runtime_backend": admission.runtime_backend,
        "verification_id": admission.verification_id,
        "runtime_verification_fingerprint": admission.runtime_verification_fingerprint,
        "runtime_observation_hash": admission.runtime_observation_hash,
        "evaluated_at": admission.evaluated_at,
    })
}

#[allow(clippy::too_many_arguments)]
async fn withhold_assignment(
    transaction: &Transaction<'_>,
    request_id: &str,
    authorized: &AuthorizedSession,
    lease_id: &str,
    job_id: &str,
    failure_reason: &str,
    reason_codes: &[String],
    admission: Option<&RuntimeAdmissionDecision>,
    now: &str,
) -> Result<(), SessionError> {
    let reason_codes_json = serde_json::to_string(reason_codes)
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    let updated_lease = transaction
        .execute(
            "UPDATE job_leases SET status = 'expired', reason_codes_json = $1, failure_reason = $2, updated_at = $3 WHERE lease_id = $4 AND status = 'offered'",
            &[&reason_codes_json, &failure_reason, &now, &lease_id],
        )
        .await?;
    if updated_lease != 1 {
        return Err(SessionError::Conflict(
            "offered lease changed before assignment could be withheld".to_string(),
        ));
    }
    let updated_job = transaction
        .execute(
            "UPDATE compute_jobs SET status = 'queued', assigned_at = NULL, assignment_lease_id = NULL, job_credential_hash = NULL, job_credential_expires_at = NULL, updated_at = $1 WHERE job_id = $2 AND status = 'queued' AND assignment_lease_id IS NULL",
            &[&now, &job_id],
        )
        .await?;
    if updated_job != 1 {
        return Err(SessionError::Conflict(
            "leased job changed before assignment could be withheld".to_string(),
        ));
    }
    let audit_metadata = serde_json::json!({
        "failure_reason": failure_reason,
        "reason_codes": reason_codes,
        "runtime_admission": admission.map(runtime_admission_audit_metadata),
    })
    .to_string();
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id,
            actor_type: "device",
            actor_id: Some(authorized.device_id.clone()),
            entity_type: "job_lease",
            entity_id: lease_id,
            event_type: "lease.assignment_withheld",
            idempotency_key: None,
            summary: "job assignment withheld after fail-closed revalidation",
            metadata_json: &audit_metadata,
        },
    )
    .await?;
    Ok(())
}

struct AcceptanceWithholding<'a> {
    request_id: &'a str,
    authorized: &'a AuthorizedSession,
    job: &'a JobRecord,
    lease: &'a JobLeaseRecord,
    failure_reason: &'a str,
    reason_codes: &'a [String],
    admission: Option<&'a RuntimeAdmissionDecision>,
    now: &'a str,
}

async fn withhold_acceptance(
    transaction: &Transaction<'_>,
    withholding: AcceptanceWithholding<'_>,
) -> Result<(), SessionError> {
    let reason_codes_json = serde_json::to_string(withholding.reason_codes)
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    if matches!(
        withholding.lease.status.as_str(),
        "offered" | "accepted" | "provisioning" | "active"
    ) {
        let updated_lease = transaction
            .execute(
                "UPDATE job_leases SET status = 'expired', reason_codes_json = $1, failure_reason = $2, updated_at = $3 WHERE lease_id = $4 AND status IN ('offered', 'accepted', 'provisioning', 'active')",
                &[
                    &reason_codes_json,
                    &withholding.failure_reason,
                    &withholding.now,
                    &withholding.lease.lease_id,
                ],
            )
            .await?;
        if updated_lease != 1 {
            return Err(SessionError::Conflict(
                "active lease changed before acceptance could be withheld".to_string(),
            ));
        }
    }
    let updated_job = transaction
        .execute(
            "UPDATE compute_jobs SET status = 'queued', assigned_at = NULL, accepted_at = NULL, assignment_lease_id = NULL, job_credential_hash = NULL, job_credential_expires_at = NULL, updated_at = $1 WHERE job_id = $2 AND status = 'assigned' AND assignment_lease_id = $3",
            &[
                &withholding.now,
                &withholding.job.job_id,
                &withholding.lease.lease_id,
            ],
        )
        .await?;
    if updated_job != 1 {
        return Err(SessionError::Conflict(
            "assigned job changed before acceptance could be withheld".to_string(),
        ));
    }
    let audit_metadata = serde_json::json!({
        "failure_reason": withholding.failure_reason,
        "reason_codes": withholding.reason_codes,
        "lease_id": withholding.lease.lease_id,
        "runtime_admission": withholding.admission.map(runtime_admission_audit_metadata),
    })
    .to_string();
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id: withholding.request_id,
            actor_type: "device",
            actor_id: Some(withholding.authorized.device_id.clone()),
            entity_type: "job_lease",
            entity_id: &withholding.lease.lease_id,
            event_type: "lease.acceptance_withheld",
            idempotency_key: None,
            summary: "job acceptance withheld after fail-closed revalidation",
            metadata_json: &audit_metadata,
        },
    )
    .await?;
    Ok(())
}

fn provider_job_execution_spec(
    job: &JobRecord,
    lease: &JobLeaseRecord,
    data_plane: &JobDataPlaneGrant,
) -> Result<ProviderJobExecutionSpec, SessionError> {
    let spec = ProviderJobExecutionSpec {
        schema_version: PROVIDER_JOB_EXECUTION_SCHEMA_VERSION.to_string(),
        policy_version: PROVIDER_JOB_EXECUTION_POLICY_VERSION.to_string(),
        job_schema_version: job.schema_version.clone(),
        lease_schema_version: lease.schema_version.clone(),
        data_plane_schema_version: data_plane.schema_version.clone(),
        job_id: job.job_id.clone(),
        lease_id: lease.lease_id.clone(),
        provider_id: job.provider_id.clone(),
        device_id: job.device_id.clone(),
        session_id: job.session_id.clone(),
        workload_type: job.workload_type.clone(),
        template_id: job.template_id.clone(),
        image_ref: job.image_ref.clone(),
        gpu_uuid: job.gpu_uuid.clone(),
        backend: job.backend.clone(),
        policy_id: job.policy_id.clone(),
        workload_policy_version: job.policy_version.clone(),
        initial_state: ProviderJobExecutionState::Assigned,
        timeout_seconds: job.timeout_seconds,
        lease_expires_at: lease.expires_at.clone(),
        data_plane_credential_expires_at: data_plane.credential_expires_at.clone(),
        runtime: ProviderJobRuntimePolicy::v2(),
        cancellation: ProviderJobCancellationPolicy::v1(),
        cleanup: ProviderJobCleanupPolicy::v1(),
    };
    validate_provider_job_execution_bundle(job, lease, data_plane, &spec).map_err(|_| {
        SessionError::Conflict("provider job execution bundle is inconsistent".to_string())
    })?;
    Ok(spec)
}
fn data_plane_grant(
    job: &JobRecord,
    credential: String,
    credential_expires_at: String,
) -> JobDataPlaneGrant {
    let download_urls = job
        .input_artifacts
        .iter()
        .map(|artifact| JobDataPlaneUrl {
            artifact_id: artifact.artifact_id.clone(),
            method: "GET".to_string(),
            url: format!(
                "/v1/jobs/{}/artifacts/{}/download",
                job.job_id, artifact.artifact_id
            ),
            expires_at: credential_expires_at.clone(),
        })
        .collect();
    let upload_urls = job
        .expected_outputs
        .iter()
        .map(|artifact| JobDataPlaneUrl {
            artifact_id: artifact.artifact_id.clone(),
            method: "PUT".to_string(),
            url: format!(
                "/v1/jobs/{}/results/{}/upload",
                job.job_id, artifact.artifact_id
            ),
            expires_at: credential_expires_at.clone(),
        })
        .collect();
    JobDataPlaneGrant {
        schema_version: JOB_DATA_PLANE_GRANT_VERSION.to_string(),
        job_id: job.job_id.clone(),
        credential,
        credential_expires_at,
        download_urls,
        upload_urls,
    }
}

fn validate_create_job_request(request: &CreateJobRequest) -> Result<(), SessionError> {
    if let Some(client_job_id) = request.client_job_id.as_deref() {
        validate_id("client_job_id", client_job_id, 128)?;
    }
    for (label, value, max_len) in [
        ("provider_id", request.provider_id.as_str(), 128),
        ("device_id", request.device_id.as_str(), 128),
        ("session_id", request.session_id.as_str(), 128),
        ("workload_type", request.workload_type.as_str(), 96),
        ("template_id", request.template_id.as_str(), 64),
        ("gpu_uuid", request.gpu_uuid.as_str(), 128),
        ("backend", request.backend.as_str(), 64),
    ] {
        validate_id(label, value, max_len)?;
    }
    if !PROVIDER_JOB_APPROVED_TEMPLATES
        .iter()
        .any(|template| *template == request.template_id)
    {
        return Err(SessionError::Invalid(
            "job template is not approved".to_string(),
        ));
    }
    validate_image_ref(&request.image_ref)?;
    if request.backend != "cuda" {
        return Err(SessionError::Invalid(
            "BN-13 jobs initially require cuda backend".to_string(),
        ));
    }
    if let Some(timeout) = request.timeout_seconds
        && (timeout == 0 || timeout > MAX_JOB_TIMEOUT_SECONDS)
    {
        return Err(SessionError::Invalid(
            "job timeout_seconds is outside allowed range".to_string(),
        ));
    }
    if let Some(policy_id) = request.policy_id.as_deref() {
        validate_id("policy_id", policy_id, 128)?;
    }
    if let Some(policy_version) = request.policy_version.as_deref() {
        validate_id("policy_version", policy_version, 64)?;
    }
    normalized_json_object(&request.parameters, "job parameters")?;
    validate_artifacts(&request.input_artifacts, "input_artifacts")?;
    validate_artifacts(&request.expected_outputs, "expected_outputs")?;
    validate_transfer_manifests(&request.input_artifacts, &request.expected_outputs)
}

fn validate_transfer_manifests(
    inputs: &[JobArtifact],
    outputs: &[JobArtifact],
) -> Result<(), SessionError> {
    let mut ids = std::collections::HashSet::new();
    let mut total_input = 0_u64;
    let mut total_output = 0_u64;
    for artifact in inputs {
        if artifact.role != "input"
            || artifact.sha256.is_none()
            || artifact.size_bytes.is_none()
            || !ids.insert(artifact.artifact_id.as_str())
        {
            return Err(SessionError::Invalid(
                "input artifacts require unique IDs, input role, size_bytes, and sha256"
                    .to_string(),
            ));
        }
        total_input = total_input
            .checked_add(artifact.size_bytes.unwrap_or_default())
            .filter(|total| *total <= 10 * 1024 * 1024 * 1024)
            .ok_or_else(|| {
                SessionError::Invalid("total input artifact size exceeds limit".to_string())
            })?;
    }
    ids.clear();
    for artifact in outputs {
        if artifact.role != "output"
            || artifact.size_bytes.is_none()
            || !ids.insert(artifact.artifact_id.as_str())
        {
            return Err(SessionError::Invalid(
                "expected outputs require unique IDs, output role, and a size_bytes limit"
                    .to_string(),
            ));
        }
        total_output = total_output
            .checked_add(artifact.size_bytes.unwrap_or_default())
            .filter(|total| *total <= 10 * 1024 * 1024 * 1024)
            .ok_or_else(|| {
                SessionError::Invalid("total output artifact size exceeds limit".to_string())
            })?;
    }
    Ok(())
}

fn validate_job_event_request(request: &JobEventRequest) -> Result<(), SessionError> {
    if request.sequence == 0 || request.sequence > i64::MAX as u64 {
        return Err(SessionError::Invalid(
            "job event sequence must be between 1 and i64::MAX".to_string(),
        ));
    }
    match request.event_type.as_str() {
        "provisioning" | "started" | "running" | "uploading" | "progress" | "log"
        | "cleanup_completed" => {}
        _ => {
            return Err(SessionError::Invalid(
                "unsupported job event_type".to_string(),
            ));
        }
    }
    validate_progress(request.progress_percent)?;
    validate_job_message(request.message.as_deref())?;
    normalized_json_object(&request.metadata, "job event metadata")?;
    if let Some(occurred_at) = request.occurred_at.as_deref() {
        validate_timestamp(occurred_at)?;
    }
    Ok(())
}

fn validate_job_result_request(request: &SubmitJobResultRequest) -> Result<(), SessionError> {
    if !matches!(request.status.as_str(), "succeeded" | "failed") {
        return Err(SessionError::Invalid(
            "job result status must be succeeded or failed".to_string(),
        ));
    }
    validate_artifacts(&request.result_artifacts, "result_artifacts")?;
    normalized_json_object(&request.metrics, "job result metrics")?;
    if let Some(code) = request.error_code.as_deref() {
        validate_id("error_code", code, 64)?;
    }
    validate_job_message(request.error_message.as_deref())?;
    if let Some(completed_at) = request.completed_at.as_deref() {
        validate_timestamp(completed_at)?;
    }
    Ok(())
}

fn validate_artifacts(artifacts: &[JobArtifact], label: &str) -> Result<(), SessionError> {
    if artifacts.len() > MAX_JOB_ARTIFACTS {
        return Err(SessionError::Invalid(format!(
            "{label} exceeds maximum artifact count"
        )));
    }
    for artifact in artifacts {
        validate_id("artifact_id", &artifact.artifact_id, 128)?;
        if artifact.artifact_id.starts_with('.') {
            return Err(SessionError::Invalid(
                "artifact_id must not use a reserved hidden name".to_string(),
            ));
        }
        validate_id("artifact_role", &artifact.role, 64)?;
        if !is_bounded_ascii(&artifact.object_key, 256)
            || artifact.object_key.contains("..")
            || artifact.object_key.contains("://")
            || artifact.object_key.starts_with('/')
            || contains_secret_text(&artifact.object_key)
        {
            return Err(SessionError::Invalid(
                "artifact object_key must be a redacted object-storage key".to_string(),
            ));
        }
        if let Some(sha256) = artifact.sha256.as_deref()
            && !is_sha256_digest(sha256)
        {
            return Err(SessionError::Invalid(
                "artifact sha256 must use sha256:<64 hex> format".to_string(),
            ));
        }
        if let Some(size) = artifact.size_bytes
            && size > 10 * 1024 * 1024 * 1024
        {
            return Err(SessionError::Invalid(
                "artifact size_bytes exceeds BN-13 limit".to_string(),
            ));
        }
        if let Some(content_type) = artifact.content_type.as_deref()
            && !is_bounded_ascii(content_type, 128)
        {
            return Err(SessionError::Invalid(
                "artifact content_type must be short printable ASCII".to_string(),
            ));
        }
    }
    Ok(())
}

fn artifacts_json(artifacts: &[JobArtifact]) -> Result<String, SessionError> {
    validate_artifacts(artifacts, "artifacts")?;
    serde_json::to_string(artifacts).map_err(|error| SessionError::Invalid(error.to_string()))
}

fn normalized_json_object(value: &serde_json::Value, label: &str) -> Result<String, SessionError> {
    if value.is_null() {
        return Ok("{}".to_string());
    }
    if !value.is_object() {
        return Err(SessionError::Invalid(format!(
            "{label} must be a JSON object"
        )));
    }
    if contains_secret_field(value) {
        return Err(SessionError::Invalid(format!(
            "{label} must not contain secret fields"
        )));
    }
    serde_json::to_string(value).map_err(|error| SessionError::Invalid(error.to_string()))
}

fn validate_image_ref(value: &str) -> Result<(), SessionError> {
    let digest = value.rsplit_once('@').map(|(_, digest)| digest);
    if !is_bounded_ascii(value, 256)
        || contains_secret_text(value)
        || !digest.is_some_and(is_sha256_digest)
    {
        return Err(SessionError::Invalid(
            "job image_ref must be a digest-pinned, redacted image reference".to_string(),
        ));
    }
    Ok(())
}

fn is_sha256_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.chars().all(|character| character.is_ascii_hexdigit())
}

fn validate_progress(value: Option<f64>) -> Result<(), SessionError> {
    if let Some(value) = value
        && (!value.is_finite() || !(0.0..=100.0).contains(&value))
    {
        return Err(SessionError::Invalid(
            "progress_percent must be finite and between 0 and 100".to_string(),
        ));
    }
    Ok(())
}

fn validate_job_message(value: Option<&str>) -> Result<(), SessionError> {
    if let Some(value) = value
        && !is_bounded_ascii(value, MAX_JOB_MESSAGE_LEN)
    {
        return Err(SessionError::Invalid(
            "job message must be short printable ASCII".to_string(),
        ));
    }
    Ok(())
}

fn validate_timestamp(value: &str) -> Result<(), SessionError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|error| SessionError::Invalid(format!("invalid timestamp: {error}")))
}

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

async fn release_customer_placement(
    transaction: &Transaction<'_>,
    job_id: &str,
) -> Result<(), SessionError> {
    transaction
        .execute(
            "UPDATE compute_placements SET status = 'released' WHERE placement_id = (SELECT placement_id FROM compute_jobs WHERE job_id = $1) AND status = 'selected'",
            &[&job_id],
        )
        .await?;
    transaction
        .execute(
            "UPDATE marketplace_reservations SET status = 'released', updated_at = $1 WHERE reservation_id = (SELECT reservation_id FROM compute_jobs WHERE job_id = $2) AND status = 'consumed'",
            &[&Utc::now().to_rfc3339(), &job_id],
        )
        .await?;
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
    let exact_or_suffix_token = lower == "token"
        || lower.ends_with("_token")
        || lower.ends_with("-token")
        || lower.ends_with(".token");
    exact_or_suffix_token
        || [
            "password",
            "secret",
            "private_key",
            "api_key",
            "authorization",
            "credential",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn job_from_row(row: Row) -> Result<JobRecord, SessionError> {
    let parameters_json: String = row.get("parameters_json");
    let input_artifacts_json: String = row.get("input_artifacts_json");
    let expected_outputs_json: String = row.get("expected_outputs_json");
    let result_artifacts_json: String = row.get("result_artifacts_json");
    Ok(JobRecord {
        job_id: row.get("job_id"),
        client_job_id: row.get("client_job_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        schema_version: row.get("schema_version"),
        workload_type: row.get("workload_type"),
        template_id: row.get("template_id"),
        image_ref: row.get("image_ref"),
        gpu_uuid: row.get("gpu_uuid"),
        backend: row.get("backend"),
        parameters: serde_json::from_str(&parameters_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        input_artifacts: serde_json::from_str(&input_artifacts_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        expected_outputs: serde_json::from_str(&expected_outputs_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        result_artifacts: serde_json::from_str(&result_artifacts_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
        status: row.get("status"),
        progress_percent: row.get("progress_percent"),
        status_message: row.get("status_message"),
        error_code: row.get("error_code"),
        error_message: row.get("error_message"),
        cancellation_reason: row.get("cancellation_reason"),
        timeout_seconds: row.get::<_, i32>("timeout_seconds") as u32,
        created_at: row.get("created_at"),
        assigned_at: row.get("assigned_at"),
        accepted_at: row.get("accepted_at"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        updated_at: row.get("updated_at"),
    })
}

fn event_from_row(row: Row) -> Result<JobEventRecord, SessionError> {
    let metadata_json: String = row.get("metadata_json");
    Ok(JobEventRecord {
        event_id: row.get("event_id"),
        job_id: row.get("job_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        sequence: row.get::<_, i64>("sequence") as u64,
        schema_version: row.get("schema_version"),
        event_type: row.get("event_type"),
        progress_percent: row.get("progress_percent"),
        message: row.get("message"),
        metadata: serde_json::from_str(&metadata_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        occurred_at: row.get("occurred_at"),
        server_received_at: row.get("server_received_at"),
    })
}

fn idempotency_from_row(row: Row) -> IdempotencyRecord {
    IdempotencyRecord {
        request_hash: row.get("request_hash"),
        status_code: row.get::<_, i32>("status_code") as u16,
        response_json: row.get("response_json"),
    }
}

fn job_select_columns() -> &'static str {
    "SELECT job_id, client_job_id, provider_id, device_id, session_id, schema_version, workload_type, template_id, image_ref, gpu_uuid, backend, parameters_json, input_artifacts_json, expected_outputs_json, result_artifacts_json, policy_id, policy_version, status, progress_percent, status_message, error_code, error_message, cancellation_reason, timeout_seconds, created_at, assigned_at, accepted_at, started_at, completed_at, updated_at FROM compute_jobs"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_job_request() -> CreateJobRequest {
        CreateJobRequest {
            client_job_id: Some("client_job_1".to_string()),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            workload_type: "llm_realtime_api".to_string(),
            template_id: "llm_inference".to_string(),
            image_ref: "ghcr.io/burd/runtime/llm@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            backend: "cuda".to_string(),
            parameters: serde_json::json!({"prompt_tokens": 128}),
            input_artifacts: vec![JobArtifact {
                artifact_id: "prompt".to_string(),
                role: "input".to_string(),
                object_key: "jobs/client_job_1/prompt.json".to_string(),
                sha256: Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                size_bytes: Some(1024),
                content_type: Some("application/json".to_string()),
            }],
            expected_outputs: vec![JobArtifact {
                artifact_id: "response".to_string(),
                role: "output".to_string(),
                object_key: "jobs/client_job_1/response.json".to_string(),
                sha256: None,
                size_bytes: Some(2048),
                content_type: Some("application/json".to_string()),
            }],
            timeout_seconds: Some(900),
            policy_id: Some("llm_realtime_api_cuda".to_string()),
            policy_version: Some("2026.07.0".to_string()),
        }
    }

    async fn assignment_fixture(
        prefix: &str,
        gpu_uuids: &[&str],
    ) -> (
        Database,
        AuthorizedSession,
        crate::runtime_admission::RuntimeAdmissionPolicy,
        chrono::DateTime<Utc>,
    ) {
        assert!(!gpu_uuids.is_empty());
        let db = crate::scheduler::tests::postgres_test_database(prefix).await;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let expires_at = (now + Duration::hours(1)).to_rfc3339();
        let client = db.connect().await.unwrap();
        crate::scheduler::tests::seed_provider_and_policy(&client, "provider_1", &now_text).await;
        crate::scheduler::tests::seed_device(
            &client,
            "provider_1",
            "device_1",
            "session_1",
            &now_text,
            &expires_at,
        )
        .await;
        let gpu_uuids = gpu_uuids
            .iter()
            .map(|gpu_uuid| (*gpu_uuid).to_string())
            .collect::<Vec<_>>();
        crate::scheduler::tests::seed_admitted_runtime(
            &client,
            "provider_1",
            "device_1",
            "session_1",
            &gpu_uuids,
            &gpu_uuids[0],
            now,
        )
        .await;
        for gpu_uuid in gpu_uuids.iter().skip(1) {
            crate::scheduler::tests::seed_additional_runtime_verification(
                &client,
                "provider_1",
                "device_1",
                "session_1",
                gpu_uuid,
                now,
            )
            .await;
        }
        drop(client);
        (
            db,
            AuthorizedSession {
                provider_id: "provider_1".to_string(),
                device_id: "device_1".to_string(),
                session_id: "session_1".to_string(),
                sequence_last: 0,
                heartbeat_interval_seconds: 15,
                missed_heartbeat_limit: 3,
            },
            crate::scheduler::tests::runtime_policy(),
            now,
        )
    }

    async fn seed_and_offer_jobs(
        db: &Database,
        policy: &crate::runtime_admission::RuntimeAdmissionPolicy,
        base_time: chrono::DateTime<Utc>,
        jobs: &[(&str, &str, i64)],
    ) {
        let client = db.connect().await.unwrap();
        for (job_id, gpu_uuid, created_offset_seconds) in jobs {
            let created_at = (base_time + Duration::seconds(*created_offset_seconds)).to_rfc3339();
            crate::scheduler::tests::seed_job(
                &client,
                job_id,
                "provider_1",
                "device_1",
                "session_1",
                gpu_uuid,
                &created_at,
            )
            .await;
        }
        drop(client);
        let scheduled = db
            .run_scheduler(
                "req_scheduler_assignment_fixture",
                &burd_protocol::RunSchedulerRequest {
                    limit: Some(jobs.len() as u32),
                    lease_ttl_seconds: Some(120),
                    reason: Some("assignment_revalidation_test".to_string()),
                },
                policy,
            )
            .await
            .unwrap();
        assert_eq!(scheduled.offered, jobs.len() as u32);
    }

    async fn insert_latest_inventory_snapshot(db: &Database, gpu_uuids: &[&str]) {
        let client = db.connect().await.unwrap();
        let observed_at = (Utc::now() + Duration::seconds(1)).to_rfc3339();
        let gpu_uuids = gpu_uuids
            .iter()
            .map(|gpu_uuid| (*gpu_uuid).to_string())
            .collect::<Vec<_>>();
        crate::scheduler::tests::seed_gpu_inventory_snapshot(
            &client,
            "provider_1",
            "device_1",
            "session_1",
            "key_device_1",
            &gpu_uuids,
            &observed_at,
        )
        .await;
    }

    async fn assert_assignment_withheld(db: &Database, job_id: &str, expected_reason_code: &str) {
        let client = db.connect().await.unwrap();
        let job = client
            .query_one(
                "SELECT status, assignment_lease_id, job_credential_hash, job_credential_expires_at FROM compute_jobs WHERE job_id = $1",
                &[&job_id],
            )
            .await
            .unwrap();
        assert_eq!(job.get::<_, String>("status"), "queued");
        assert!(
            job.get::<_, Option<String>>("assignment_lease_id")
                .is_none()
        );
        assert!(
            job.get::<_, Option<String>>("job_credential_hash")
                .is_none()
        );
        assert!(
            job.get::<_, Option<String>>("job_credential_expires_at")
                .is_none()
        );
        let lease = client
            .query_one(
                "SELECT status, failure_reason, reason_codes_json FROM job_leases WHERE job_id = $1",
                &[&job_id],
            )
            .await
            .unwrap();
        assert_eq!(lease.get::<_, String>("status"), "expired");
        assert_eq!(
            lease.get::<_, Option<String>>("failure_reason").as_deref(),
            Some("runtime_admission_lost_before_assignment")
        );
        let reason_codes_json: String = lease.get("reason_codes_json");
        let reason_codes: Vec<String> = serde_json::from_str(&reason_codes_json).unwrap();
        assert!(
            reason_codes
                .iter()
                .any(|reason| reason == expected_reason_code)
        );
        let audit_metadata: String = client
            .query_one(
                "SELECT metadata_json FROM audit_events WHERE entity_type = 'job_lease' AND entity_id = (SELECT lease_id FROM job_leases WHERE job_id = $1) AND event_type = 'lease.assignment_withheld' ORDER BY occurred_at DESC LIMIT 1",
                &[&job_id],
            )
            .await
            .unwrap()
            .get("metadata_json");
        assert!(audit_metadata.contains(expected_reason_code));
        assert!(!audit_metadata.contains("jobcred_"));
    }

    async fn assert_acceptance_withheld(
        db: &Database,
        job_id: &str,
        expected_reason_code: &str,
        expected_lease_failure_reason: &str,
    ) {
        let client = db.connect().await.unwrap();
        let job = client
            .query_one(
                "SELECT status, job_credential_hash, job_credential_expires_at FROM compute_jobs WHERE job_id = $1",
                &[&job_id],
            )
            .await
            .unwrap();
        assert_eq!(job.get::<_, String>("status"), "queued");
        assert!(
            job.get::<_, Option<String>>("job_credential_hash")
                .is_none()
        );
        assert!(
            job.get::<_, Option<String>>("job_credential_expires_at")
                .is_none()
        );
        let lease = client
            .query_one(
                "SELECT status, failure_reason FROM job_leases WHERE job_id = $1 ORDER BY offered_at DESC LIMIT 1",
                &[&job_id],
            )
            .await
            .unwrap();
        assert_eq!(lease.get::<_, String>("status"), "expired");
        assert_eq!(
            lease.get::<_, Option<String>>("failure_reason").as_deref(),
            Some(expected_lease_failure_reason)
        );
        let audit_metadata: String = client
            .query_one(
                "SELECT metadata_json FROM audit_events WHERE entity_type = 'job_lease' AND entity_id = (SELECT lease_id FROM job_leases WHERE job_id = $1 ORDER BY offered_at DESC LIMIT 1) AND event_type = 'lease.acceptance_withheld' ORDER BY occurred_at DESC LIMIT 1",
                &[&job_id],
            )
            .await
            .unwrap()
            .get("metadata_json");
        assert!(audit_metadata.contains(expected_reason_code));
        assert!(!audit_metadata.contains("jobcred_"));
    }

    #[test]
    fn validation_rejects_shell_like_or_unpinned_jobs() {
        assert!(validate_create_job_request(&create_job_request()).is_ok());

        let mut bad_template = create_job_request();
        bad_template.template_id = "shell".to_string();
        assert!(validate_create_job_request(&bad_template).is_err());

        let mut bad_image = create_job_request();
        bad_image.image_ref = "ghcr.io/burd/runtime/latest".to_string();
        assert!(validate_create_job_request(&bad_image).is_err());

        let mut secret = create_job_request();
        secret.parameters = serde_json::json!({"api_token": "leak"});
        assert!(validate_create_job_request(&secret).is_err());

        let mut reserved_artifact = create_job_request();
        reserved_artifact.input_artifacts[0].artifact_id = ".burd-placeholder".to_string();
        assert!(validate_create_job_request(&reserved_artifact).is_err());
    }

    #[test]
    fn data_plane_grant_uses_job_scoped_paths_without_raw_tokens() {
        let request = create_job_request();
        let job = JobRecord {
            job_id: "job_1".to_string(),
            client_job_id: None,
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            schema_version: JOB_SCHEMA_VERSION.to_string(),
            workload_type: "llm_realtime_api".to_string(),
            template_id: "llm_inference".to_string(),
            image_ref: "ghcr.io/burd/runtime/llm@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            backend: "cuda".to_string(),
            parameters: serde_json::json!({}),
            input_artifacts: request.input_artifacts,
            expected_outputs: request.expected_outputs,
            result_artifacts: Vec::new(),
            policy_id: None,
            policy_version: None,
            status: "assigned".to_string(),
            progress_percent: None,
            status_message: None,
            error_code: None,
            error_message: None,
            cancellation_reason: None,
            timeout_seconds: 900,
            created_at: "2026-07-13T00:00:00Z".to_string(),
            assigned_at: None,
            accepted_at: None,
            started_at: None,
            completed_at: None,
            updated_at: "2026-07-13T00:00:00Z".to_string(),
        };
        let grant = data_plane_grant(
            &job,
            "jobcred_example".to_string(),
            "2026-07-13T01:00:00Z".to_string(),
        );
        assert_eq!(grant.download_urls.len(), 1);
        assert_eq!(grant.upload_urls.len(), 1);
        assert!(!grant.download_urls[0].url.contains("jobcred_example"));
        assert!(grant.download_urls[0].url.contains("/download"));

        let lease = JobLeaseRecord {
            lease_id: "lease_1".to_string(),
            job_id: job.job_id.clone(),
            provider_id: job.provider_id.clone(),
            device_id: job.device_id.clone(),
            session_id: job.session_id.clone(),
            schema_version: burd_protocol::JOB_LEASE_SCHEMA_VERSION.to_string(),
            workload_type: job.workload_type.clone(),
            gpu_uuid: job.gpu_uuid.clone(),
            policy_id: job.policy_id.clone(),
            policy_version: job.policy_version.clone(),
            status: "offered".to_string(),
            reason_codes: Vec::new(),
            offered_at: "2026-07-13T00:00:00Z".to_string(),
            expires_at: "2026-07-13T00:05:00Z".to_string(),
            accepted_at: None,
            provisioning_at: None,
            active_at: None,
            completed_at: None,
            failure_reason: None,
            created_at: "2026-07-13T00:00:00Z".to_string(),
            updated_at: "2026-07-13T00:00:00Z".to_string(),
        };
        let execution = provider_job_execution_spec(&job, &lease, &grant).unwrap();
        assert_eq!(execution.lease_id, lease.lease_id);
        assert_eq!(execution.gpu_uuid, job.gpu_uuid);
        assert!(
            !serde_json::to_string(&execution)
                .unwrap()
                .contains(&grant.credential)
        );

        let mut wrong_lease = lease;
        wrong_lease.job_id = "job_2".to_string();
        assert!(provider_job_execution_spec(&job, &wrong_lease, &grant).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_assignment_revalidation_withholds_after_authority_changes() {
        for (scenario, expected_reason) in [
            ("observation_stale", "runtime_observation_stale"),
            ("key_revoked", "active_device_key_missing"),
            ("gpu_removed", "gpu_inventory_missing"),
            ("provider_blocked", "provider_not_active"),
            ("device_blocked", "device_not_active"),
            ("verification_expired", "runtime_verification_expired"),
        ] {
            let gpu_uuids = if scenario == "gpu_removed" {
                vec!["GPU-A", "GPU-B"]
            } else {
                vec!["GPU-A"]
            };
            let (db, authorized, mut policy, now) =
                assignment_fixture(&format!("burd_assignment_{scenario}"), &gpu_uuids).await;
            seed_and_offer_jobs(&db, &policy, now, &[("job_denied", "GPU-A", -10)]).await;

            if scenario == "observation_stale" {
                policy.observation_max_age_seconds = 0;
            } else if scenario == "gpu_removed" {
                insert_latest_inventory_snapshot(&db, &[]).await;
            } else {
                let client = db.connect().await.unwrap();
                match scenario {
                    "key_revoked" => {
                        client
                            .execute(
                                "UPDATE provider_public_keys SET status = 'revoked' WHERE public_key_id = 'key_device_1'",
                                &[],
                            )
                            .await
                            .unwrap();
                    }
                    "provider_blocked" => {
                        client
                            .execute(
                                "UPDATE providers SET status = 'blocked' WHERE provider_id = 'provider_1'",
                                &[],
                            )
                            .await
                            .unwrap();
                    }
                    "device_blocked" => {
                        client
                            .execute(
                                "UPDATE devices SET status = 'blocked' WHERE device_id = 'device_1'",
                                &[],
                            )
                            .await
                            .unwrap();
                    }
                    "verification_expired" => {
                        let row = client
                            .query_one(
                                "SELECT verification_id, record_json FROM provider_runtime_verifications WHERE provider_id = 'provider_1' AND device_id = 'device_1' AND gpu_uuid = 'GPU-A' ORDER BY verified_at DESC LIMIT 1",
                                &[],
                            )
                            .await
                            .unwrap();
                        let verification_id: String = row.get("verification_id");
                        let record_json: String = row.get("record_json");
                        let mut record: burd_protocol::ProviderRuntimeVerificationRecord =
                            serde_json::from_str(&record_json).unwrap();
                        record.expires_at = (now - Duration::seconds(1)).to_rfc3339();
                        let expired_record_json = serde_json::to_string(&record).unwrap();
                        client
                            .execute(
                                "UPDATE provider_runtime_verifications SET expires_at = $1, record_json = $2 WHERE verification_id = $3",
                                &[&record.expires_at, &expired_record_json, &verification_id],
                            )
                            .await
                            .unwrap();
                    }
                    _ => unreachable!(),
                }
            }

            let next = db
                .next_job_for_session("req_assignment_denied", &authorized, &policy)
                .await
                .unwrap();
            assert!(next.job.is_none());
            assert!(next.data_plane.is_none());
            assert!(next.lease.is_none());
            assert!(next.execution.is_none());
            assert_assignment_withheld(&db, "job_denied", expected_reason).await;
        }
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_assignment_revalidation_skips_denied_offer_before_valid_offer() {
        let (db, authorized, policy, now) =
            assignment_fixture("burd_assignment_head_of_line", &["GPU-A", "GPU-B"]).await;
        seed_and_offer_jobs(
            &db,
            &policy,
            now,
            &[("job_gpu_a", "GPU-A", -10), ("job_gpu_b", "GPU-B", -5)],
        )
        .await;
        let client = db.connect().await.unwrap();
        client
            .execute(
                "UPDATE job_leases SET offered_at = CASE job_id WHEN 'job_gpu_a' THEN $1 ELSE $2 END",
                &[
                    &(now - Duration::seconds(10)).to_rfc3339(),
                    &(now - Duration::seconds(5)).to_rfc3339(),
                ],
            )
            .await
            .unwrap();
        drop(client);
        insert_latest_inventory_snapshot(&db, &["GPU-B"]).await;

        let next = db
            .next_job_for_session("req_assignment_head_of_line", &authorized, &policy)
            .await
            .unwrap();
        assert_eq!(next.job.as_ref().unwrap().job_id, "job_gpu_b");
        assert_eq!(next.job.as_ref().unwrap().status, "assigned");
        assert_eq!(next.execution.as_ref().unwrap().gpu_uuid, "GPU-B");
        let credential = next.data_plane.as_ref().unwrap().credential.clone();
        assert!(credential.starts_with("jobcred_"));
        assert_assignment_withheld(&db, "job_gpu_a", "gpu_inventory_missing").await;

        let client = db.connect().await.unwrap();
        let row = client
            .query_one(
                "SELECT job_credential_hash, job_credential_expires_at FROM compute_jobs WHERE job_id = 'job_gpu_b'",
                &[],
            )
            .await
            .unwrap();
        let expected_hash = sha256_hex(credential.as_bytes());
        assert_eq!(
            row.get::<_, Option<String>>("job_credential_hash")
                .as_deref(),
            Some(expected_hash.as_str())
        );
        assert!(
            row.get::<_, Option<String>>("job_credential_expires_at")
                .is_some()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_assignment_uses_newer_proof_without_persisting_plaintext_credential() {
        let (db, authorized, policy, now) =
            assignment_fixture("burd_assignment_newer_proof", &["GPU-A"]).await;
        seed_and_offer_jobs(&db, &policy, now, &[("job_newer_proof", "GPU-A", -10)]).await;
        let client = db.connect().await.unwrap();
        let scheduler_audit: String = client
            .query_one(
                "SELECT metadata_json FROM audit_events WHERE event_type = 'lease.offered' AND entity_id = (SELECT lease_id FROM job_leases WHERE job_id = 'job_newer_proof') ORDER BY occurred_at DESC LIMIT 1",
                &[],
            )
            .await
            .unwrap()
            .get("metadata_json");
        drop(client);
        let client = db.connect().await.unwrap();
        let newer_verification_id = crate::scheduler::tests::seed_additional_runtime_verification(
            &client,
            "provider_1",
            "device_1",
            "session_1",
            "GPU-A",
            now + Duration::seconds(30),
        )
        .await;
        drop(client);
        assert!(!scheduler_audit.contains(&newer_verification_id));

        let next = db
            .next_job_for_session("req_assignment_newer_proof", &authorized, &policy)
            .await
            .unwrap();
        let credential = next.data_plane.as_ref().unwrap().credential.clone();
        let credential_hash = sha256_hex(credential.as_bytes());
        let client = db.connect().await.unwrap();
        let persisted_hash: Option<String> = client
            .query_one(
                "SELECT job_credential_hash FROM compute_jobs WHERE job_id = 'job_newer_proof'",
                &[],
            )
            .await
            .unwrap()
            .get("job_credential_hash");
        assert_eq!(persisted_hash.as_deref(), Some(credential_hash.as_str()));
        assert_ne!(persisted_hash.as_deref(), Some(credential.as_str()));
        let audit_rows = client
            .query(
                "SELECT metadata_json FROM audit_events WHERE entity_id IN ('job_newer_proof', (SELECT lease_id FROM job_leases WHERE job_id = 'job_newer_proof'))",
                &[],
            )
            .await
            .unwrap();
        let mut assignment_used_newer_proof = false;
        for row in audit_rows {
            let metadata: String = row.get("metadata_json");
            assert!(!metadata.contains(&credential));
            if metadata.contains(&newer_verification_id) {
                assignment_used_newer_proof = true;
            }
        }
        assert!(assignment_used_newer_proof);
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_acceptance_revalidation_withholds_after_authority_changes() {
        for (scenario, expected_reason, expected_lease_failure_reason) in [
            (
                "lease_expired_by_sweep",
                "lease_expired_before_acceptance",
                "lease_ack_timeout",
            ),
            (
                "lease_ttl_elapsed",
                "lease_expired_before_acceptance",
                "lease_expired_before_acceptance",
            ),
            (
                "gpu_binding_changed",
                "lease_job_binding_mismatch_before_acceptance",
                "lease_job_binding_mismatch_before_acceptance",
            ),
            (
                "key_revoked",
                "active_device_key_missing",
                "runtime_admission_lost_before_acceptance",
            ),
            (
                "gpu_inventory_empty",
                "gpu_inventory_missing",
                "runtime_admission_lost_before_acceptance",
            ),
        ] {
            let (db, authorized, policy, now) =
                assignment_fixture(&format!("burd_acceptance_{scenario}"), &["GPU-A"]).await;
            let job_id = format!("job_{scenario}");
            seed_and_offer_jobs(&db, &policy, now, &[(&job_id, "GPU-A", -10)]).await;
            let next = db
                .next_job_for_session("req_acceptance_assignment", &authorized, &policy)
                .await
                .unwrap();
            assert_eq!(next.job.as_ref().unwrap().status, "assigned");
            assert!(
                next.data_plane
                    .as_ref()
                    .unwrap()
                    .credential
                    .starts_with("jobcred_")
            );
            let lease_id = next.lease.as_ref().unwrap().lease_id.clone();

            if scenario == "gpu_inventory_empty" {
                insert_latest_inventory_snapshot(&db, &[]).await;
            } else {
                let client = db.connect().await.unwrap();
                match scenario {
                    "lease_expired_by_sweep" => {
                        client
                            .execute(
                                "UPDATE job_leases SET status = 'expired', failure_reason = 'lease_ack_timeout' WHERE job_id = $1",
                                &[&job_id],
                            )
                            .await
                            .unwrap();
                    }
                    "lease_ttl_elapsed" => {
                        client
                            .execute(
                                "UPDATE job_leases SET expires_at = $1 WHERE job_id = $2",
                                &[&(Utc::now() - Duration::seconds(1)).to_rfc3339(), &job_id],
                            )
                            .await
                            .unwrap();
                    }
                    "gpu_binding_changed" => {
                        client
                            .execute(
                                "UPDATE job_leases SET gpu_uuid = 'GPU-other' WHERE job_id = $1",
                                &[&job_id],
                            )
                            .await
                            .unwrap();
                    }
                    "key_revoked" => {
                        client
                            .execute(
                                "UPDATE provider_public_keys SET status = 'revoked' WHERE public_key_id = 'key_device_1'",
                                &[],
                            )
                            .await
                            .unwrap();
                    }
                    _ => unreachable!(),
                }
            }

            let error = db
                .accept_job(
                    "req_acceptance_denied",
                    &authorized,
                    &job_id,
                    &AcceptJobRequest {
                        lease_id,
                        status_message: None,
                    },
                    &policy,
                )
                .await
                .unwrap_err();
            assert!(matches!(error, SessionError::Conflict(_)));
            assert_acceptance_withheld(
                &db,
                &job_id,
                expected_reason,
                expected_lease_failure_reason,
            )
            .await;
        }
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_stale_acknowledgement_cannot_mutate_a_new_assignment() {
        let (db, authorized, policy, now) =
            assignment_fixture("burd_acceptance_stale_ack", &["GPU-A"]).await;
        let job_id = "job_stale_ack";
        seed_and_offer_jobs(&db, &policy, now, &[(job_id, "GPU-A", -10)]).await;
        let assignment_a = db
            .next_job_for_session("req_assignment_a", &authorized, &policy)
            .await
            .unwrap();
        let lease_a = assignment_a.lease.unwrap().lease_id;

        let client = db.connect().await.unwrap();
        let requeued_at = Utc::now().to_rfc3339();
        client
            .execute(
                "UPDATE job_leases SET status = 'expired', failure_reason = 'assignment_invalidated_for_test', updated_at = $1 WHERE lease_id = $2 AND status = 'offered'",
                &[&requeued_at, &lease_a],
            )
            .await
            .unwrap();
        client
            .execute(
                "UPDATE compute_jobs SET status = 'queued', assigned_at = NULL, assignment_lease_id = NULL, job_credential_hash = NULL, job_credential_expires_at = NULL, updated_at = $1 WHERE job_id = $2 AND status = 'assigned' AND assignment_lease_id = $3",
                &[&requeued_at, &job_id, &lease_a],
            )
            .await
            .unwrap();
        drop(client);

        let scheduled_b = db
            .run_scheduler(
                "req_scheduler_b",
                &burd_protocol::RunSchedulerRequest {
                    limit: Some(1),
                    lease_ttl_seconds: Some(120),
                    reason: Some("stale_ack_reassignment_test".to_string()),
                },
                &policy,
            )
            .await
            .unwrap();
        assert_eq!(scheduled_b.offered, 1);
        let assignment_b = db
            .next_job_for_session("req_assignment_b", &authorized, &policy)
            .await
            .unwrap();
        let lease_b = assignment_b.lease.as_ref().unwrap().lease_id.clone();
        let credential_b = assignment_b.data_plane.as_ref().unwrap().credential.clone();
        let credential_b_hash = sha256_hex(credential_b.as_bytes());
        assert_ne!(lease_a, lease_b);

        for (request_id, rejected_lease_id, expected_message) in [
            (
                "req_stale_ack_a",
                lease_a.as_str(),
                "stale assignment acknowledgement",
            ),
            (
                "req_missing_ack",
                "lease_missing",
                "stale or mismatched assignment acknowledgement",
            ),
        ] {
            let error = db
                .accept_job(
                    request_id,
                    &authorized,
                    job_id,
                    &AcceptJobRequest {
                        lease_id: rejected_lease_id.to_string(),
                        status_message: None,
                    },
                    &policy,
                )
                .await
                .unwrap_err();
            assert!(
                matches!(error, SessionError::Conflict(message) if message == expected_message)
            );
        }
        let stale_control = db
            .job_execution_control("req_stale_control_a", &authorized, job_id, &lease_a)
            .await
            .unwrap_err();
        assert!(
            matches!(stale_control, SessionError::Conflict(message) if message == "stale assignment acknowledgement")
        );

        let client = db.connect().await.unwrap();
        let current = client
            .query_one(
                "SELECT status, assignment_lease_id, job_credential_hash FROM compute_jobs WHERE job_id = $1",
                &[&job_id],
            )
            .await
            .unwrap();
        assert_eq!(current.get::<_, String>("status"), "assigned");
        assert_eq!(
            current
                .get::<_, Option<String>>("assignment_lease_id")
                .as_deref(),
            Some(lease_b.as_str())
        );
        assert_eq!(
            current
                .get::<_, Option<String>>("job_credential_hash")
                .as_deref(),
            Some(credential_b_hash.as_str())
        );
        let lease_b_status: String = client
            .query_one(
                "SELECT status FROM job_leases WHERE lease_id = $1",
                &[&lease_b],
            )
            .await
            .unwrap()
            .get("status");
        assert_eq!(lease_b_status, "offered");
        let audit_metadata = client
            .query(
                "SELECT metadata_json FROM audit_events WHERE entity_id = $1",
                &[&job_id],
            )
            .await
            .unwrap();
        assert!(audit_metadata.into_iter().all(|row| {
            let metadata: String = row.get("metadata_json");
            !metadata.contains(&credential_b)
        }));
        drop(client);

        let request_b = AcceptJobRequest {
            lease_id: lease_b.clone(),
            status_message: Some("provider worker accepted assignment".to_string()),
        };
        let db_right = db.clone();
        let authorized_right = authorized.clone();
        let policy_right = policy.clone();
        let request_b_right = request_b.clone();
        let (left, right) = tokio::join!(
            db.accept_job(
                "req_accept_b_left",
                &authorized,
                job_id,
                &request_b,
                &policy,
            ),
            db_right.accept_job(
                "req_accept_b_right",
                &authorized_right,
                job_id,
                &request_b_right,
                &policy_right,
            )
        );
        assert_eq!(
            [left.is_ok(), right.is_ok()]
                .into_iter()
                .filter(|ok| *ok)
                .count(),
            1
        );
        let client = db.connect().await.unwrap();
        let final_state = client
            .query_one(
                "SELECT status, assignment_lease_id FROM compute_jobs WHERE job_id = $1",
                &[&job_id],
            )
            .await
            .unwrap();
        assert_eq!(final_state.get::<_, String>("status"), "accepted");
        assert_eq!(
            final_state
                .get::<_, Option<String>>("assignment_lease_id")
                .as_deref(),
            Some(lease_b.as_str())
        );
        drop(client);

        let continued = db
            .job_execution_control("req_control_b", &authorized, job_id, &lease_b)
            .await
            .unwrap();
        assert_eq!(continued.directive, JobExecutionDirective::Continue);
        assert!(continued.reason_code.is_none());

        db.cancel_job(
            "req_cancel_b",
            job_id,
            &CancelJobRequest {
                reason: Some("admin_cancelled_test_job".to_string()),
            },
        )
        .await
        .unwrap();
        for request_id in ["req_cancel_control_b_1", "req_cancel_control_b_2"] {
            let cancelled = db
                .job_execution_control(request_id, &authorized, job_id, &lease_b)
                .await
                .unwrap();
            assert_eq!(cancelled.directive, JobExecutionDirective::Cancel);
            assert_eq!(cancelled.reason_code.as_deref(), Some("job_cancelled"));
        }
        let client = db.connect().await.unwrap();
        let credential_hash: Option<String> = client
            .query_one(
                "SELECT job_credential_hash FROM compute_jobs WHERE job_id = $1",
                &[&job_id],
            )
            .await
            .unwrap()
            .get("job_credential_hash");
        assert!(credential_hash.is_none());
        drop(client);
        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_lease_acceptance_requires_exactly_one_current_offer() {
        let db =
            crate::scheduler::tests::postgres_test_database("burd_acceptance_exact_offer").await;
        let mut client = db.connect().await.unwrap();
        let transaction = client.transaction().await.unwrap();
        let error = mark_lease_accepted_for_job(
            &transaction,
            "missing_lease",
            "missing_job",
            &Utc::now().to_rfc3339(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, SessionError::Conflict(_)));
        transaction.rollback().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_concurrent_assignment_poll_delivers_one_job_once() {
        let (db, authorized, policy, now) =
            assignment_fixture("burd_assignment_concurrent", &["GPU-A"]).await;
        seed_and_offer_jobs(&db, &policy, now, &[("job_concurrent", "GPU-A", -10)]).await;
        let db_right = db.clone();
        let authorized_right = authorized.clone();
        let policy_right = policy.clone();
        let (left, right) = tokio::join!(
            db.next_job_for_session("req_assignment_left", &authorized, &policy),
            db_right
                .next_job_for_session("req_assignment_right", &authorized_right, &policy_right,)
        );
        let responses = [left.unwrap(), right.unwrap()];
        assert_eq!(
            responses
                .iter()
                .filter(|response| response.job.is_some())
                .count(),
            1
        );
        assert_eq!(
            responses
                .iter()
                .filter(|response| response.data_plane.is_some())
                .count(),
            1
        );
        let client = db.connect().await.unwrap();
        let assigned_count: i64 = client
            .query_one(
                "SELECT COUNT(*)::BIGINT AS count FROM audit_events WHERE entity_id = 'job_concurrent' AND event_type = 'job.assigned'",
                &[],
            )
            .await
            .unwrap()
            .get("count");
        assert_eq!(assigned_count, 1);
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_job_lifecycle_assigns_events_and_results() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_job_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let now_instant = Utc::now();
        let now = now_instant.to_rfc3339();
        let expires_at = (now_instant + Duration::hours(1)).to_rfc3339();
        let client = db.connect().await.unwrap();
        client
            .execute(
                "INSERT INTO providers (provider_id, user_id, display_name, status, created_at, updated_at) VALUES ('provider_1', NULL, 'Job Provider', 'available', $1, $1)",
                &[&now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ('device_1', 'provider_1', 'machine_1', 'active', $1, $1)",
                &[&now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint) VALUES ('session_1', 'provider_1', 'device_1', 'online', 0, $1, $2, $3)",
                &[&now, &expires_at, &"a".repeat(64)],
            )
            .await
            .unwrap();
        crate::scheduler::tests::seed_admitted_runtime(
            &client,
            "provider_1",
            "device_1",
            "session_1",
            &["GPU-test".to_string()],
            "GPU-test",
            now_instant,
        )
        .await;
        client
            .execute(
                "INSERT INTO workload_policies (policy_id, policy_version, schema_version, workload_type, display_name, requirements_json, status, created_at, updated_at) VALUES ('llm_realtime_api_cuda', '2026.07.0', 'burd-workload-policy-v1', 'llm_realtime_api', 'LLM realtime CUDA', '{}', 'active', $1, $1)",
                &[&now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_workload_eligibility (provider_id, device_id, workload_type, policy_id, policy_version, schema_version, engine_version, status, reason_codes_json, session_status, latest_gpu_uuid, hardware_fingerprint, regional_reachability_json, evaluated_at, updated_at) VALUES ('provider_1', 'device_1', 'llm_realtime_api', 'llm_realtime_api_cuda', '2026.07.0', 'burd-workload-eligibility-v1', 'burd-workload-engine-v1', 'eligible', '[]', 'online', 'GPU-test', 'fp_1', '[]', $1, $1)",
                &[&now],
            )
            .await
            .unwrap();
        drop(client);

        let request = create_job_request();
        let outcome = db
            .create_job_idempotently(CreateJobCommand {
                request_id: "req_job_create".to_string(),
                scope: "POST /v1/jobs".to_string(),
                idempotency_key: "job-key-1".to_string(),
                request_hash: "hash_job_1".to_string(),
                request,
            })
            .await
            .unwrap();
        let CreateJobOutcome::Response(record) = outcome else {
            panic!("job creation must store an idempotent response");
        };
        assert_eq!(record.status_code, 201);
        let created: CreateJobResponse = serde_json::from_str(&record.response_json).unwrap();
        assert_eq!(created.job.status, "queued");

        let scheduled = db
            .run_scheduler(
                "req_scheduler",
                &burd_protocol::RunSchedulerRequest {
                    limit: Some(10),
                    lease_ttl_seconds: Some(120),
                    reason: Some("integration_test".to_string()),
                },
                &crate::runtime_admission::RuntimeAdmissionPolicy {
                    clock_skew_seconds: 300,
                    observation_max_age_seconds: 180,
                    approved_proof_image_ref: Some(format!(
                        "ghcr.io/burd/runtime-proof@sha256:{}",
                        "a".repeat(64)
                    )),
                },
            )
            .await
            .unwrap();
        assert_eq!(scheduled.offered, 1);

        let authorized = AuthorizedSession {
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            sequence_last: 0,
            heartbeat_interval_seconds: 15,
            missed_heartbeat_limit: 3,
        };
        let next = db
            .next_job_for_session(
                "req_next",
                &authorized,
                &crate::scheduler::tests::runtime_policy(),
            )
            .await
            .unwrap();
        assert_eq!(next.job.as_ref().unwrap().job_id, created.job.job_id);
        assert_eq!(next.job.as_ref().unwrap().status, "assigned");
        let job_credential = next.data_plane.as_ref().unwrap().credential.clone();
        assert!(
            next.data_plane
                .as_ref()
                .unwrap()
                .credential
                .starts_with("jobcred_")
        );

        assert_eq!(next.lease.as_ref().unwrap().status, "offered");
        let lease_id = next.lease.as_ref().unwrap().lease_id.clone();

        let duplicate_next = db
            .next_job_for_session(
                "req_next_again",
                &authorized,
                &crate::scheduler::tests::runtime_policy(),
            )
            .await
            .unwrap();
        assert!(duplicate_next.job.is_none());
        assert!(duplicate_next.data_plane.is_none());
        assert!(duplicate_next.lease.is_none());
        let accepted = db
            .accept_job(
                "req_accept",
                &authorized,
                &created.job.job_id,
                &AcceptJobRequest {
                    lease_id,
                    status_message: None,
                },
                &crate::scheduler::tests::runtime_policy(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.job.status, "accepted");
        let client = db.connect().await.unwrap();
        let accepted_lease_status: String = client
            .query_one(
                "SELECT status FROM job_leases WHERE job_id = $1 ORDER BY offered_at DESC LIMIT 1",
                &[&created.job.job_id],
            )
            .await
            .unwrap()
            .get("status");
        assert_eq!(accepted_lease_status, "accepted");
        let acceptance_audit: String = client
            .query_one(
                "SELECT metadata_json FROM audit_events WHERE entity_id = $1 AND event_type = 'job.accepted' ORDER BY occurred_at DESC LIMIT 1",
                &[&created.job.job_id],
            )
            .await
            .unwrap()
            .get("metadata_json");
        assert!(acceptance_audit.contains("runtime_admission"));
        assert!(acceptance_audit.contains("verification_id"));
        assert!(!acceptance_audit.contains("jobcred_"));
        drop(client);

        let event = db
            .record_job_event(
                "req_event",
                &authorized,
                &created.job.job_id,
                &JobEventRequest {
                    sequence: 1,
                    event_type: "running".to_string(),
                    progress_percent: Some(25.0),
                    message: Some("running".to_string()),
                    metadata: serde_json::json!({"container_id": "container_1"}),
                    occurred_at: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(event.event.sequence, 1);
        assert_eq!(event.job.status, "running");

        let uploading = db
            .record_job_event(
                "req_uploading",
                &authorized,
                &created.job.job_id,
                &JobEventRequest {
                    sequence: 2,
                    event_type: "uploading".to_string(),
                    progress_percent: None,
                    message: Some("uploading".to_string()),
                    metadata: serde_json::json!({}),
                    occurred_at: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(uploading.job.status, "uploading");
        let uploaded = db
            .record_job_artifact_upload(
                &created.job.job_id,
                "response",
                &job_credential,
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1536,
                Some("application/json"),
            )
            .await
            .unwrap();

        let result = db
            .submit_job_result(
                "req_result",
                &authorized,
                &created.job.job_id,
                &SubmitJobResultRequest {
                    status: "succeeded".to_string(),
                    result_artifacts: vec![uploaded],
                    metrics: serde_json::json!({"tokens_per_second": 42.0}),
                    error_code: None,
                    error_message: None,
                    completed_at: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.job.status, "succeeded");

        let usage = db
            .list_job_usage_ledger("req_usage", &created.job.job_id)
            .await
            .unwrap();
        assert_eq!(usage.entries.len(), 1);
        assert_eq!(usage.entries[0].receipt.job_status, "succeeded");
        assert!(usage.entries[0].receipt.lease_id.is_some());
        assert_eq!(usage.entries[0].receipt.input_bytes, 1024);
        assert_eq!(
            usage.entries[0].receipt_signature_status,
            "hash_only_backend_signature_not_configured"
        );

        let finalized_again = db
            .finalize_job_usage("req_usage_again", &created.job.job_id)
            .await
            .unwrap();
        assert!(finalized_again.duplicate);
        assert_eq!(finalized_again.entry.entry_id, usage.entries[0].entry_id);

        let listed = db
            .list_provider_jobs("req_list", "provider_1", 10)
            .await
            .unwrap();
        assert_eq!(listed.jobs.len(), 1);
        assert_eq!(listed.jobs[0].status, "succeeded");

        db.drop_schema_for_test().await.unwrap();
    }
}
