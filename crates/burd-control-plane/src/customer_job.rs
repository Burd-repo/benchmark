use crate::customer::CustomerApiKeyAuth;
use crate::db::Database;
use crate::job_control::release_customer_placement;
use crate::metering::append_usage_ledger_for_job;
use crate::remote_session::SessionError;
use crate::scheduler::mark_lease_terminal_for_job;
use burd_protocol::{
    CUSTOMER_JOB_EVENT_SCHEMA_VERSION, CUSTOMER_JOB_SCHEMA_VERSION, CancelCustomerWorkloadRequest,
    CustomerJobEventRecord, CustomerJobRecord, CustomerJobResponse, CustomerResultArtifact,
    CustomerWorkloadRecord, JobArtifact, ListCustomerJobEventsResponse,
};
use chrono::Utc;
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const MAX_CANCEL_REASON_LEN: usize = 512;

impl Database {
    pub async fn get_customer_job(
        &self,
        request_id: &str,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
        workload_id: &str,
    ) -> Result<CustomerJobResponse, SessionError> {
        require_customer_scope(auth, "workloads:read")?;
        validate_id("project_id", project_id, 128)?;
        validate_id("workload_id", workload_id, 128)?;
        assert_project_binding(auth, project_id)?;
        let client = self.connect().await?;
        let row = load_owned_customer_job(&client, auth, project_id, workload_id).await?;
        customer_job_response(request_id, row, false)
    }

    pub async fn list_customer_job_events(
        &self,
        request_id: &str,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
        workload_id: &str,
        limit: u32,
    ) -> Result<ListCustomerJobEventsResponse, SessionError> {
        require_customer_scope(auth, "workloads:read")?;
        validate_id("project_id", project_id, 128)?;
        validate_id("workload_id", workload_id, 128)?;
        assert_project_binding(auth, project_id)?;
        let limit = limit.clamp(1, 200) as i64;
        let client = self.connect().await?;
        let owned = load_owned_customer_job(&client, auth, project_id, workload_id).await?;
        let job_id: String = owned.get("job_id");
        let rows = client
            .query(
                "SELECT event_id, job_id, sequence, event_type, progress_percent, occurred_at FROM job_events WHERE job_id = $1 ORDER BY sequence ASC, event_id ASC LIMIT $2",
                &[&job_id, &limit],
            )
            .await?;
        Ok(ListCustomerJobEventsResponse {
            request_id: request_id.to_string(),
            events: rows.into_iter().map(customer_event_from_row).collect(),
        })
    }

    pub async fn cancel_customer_job(
        &self,
        request_id: &str,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
        workload_id: &str,
        request: &CancelCustomerWorkloadRequest,
    ) -> Result<CustomerJobResponse, SessionError> {
        require_customer_scope(auth, "workloads:write")?;
        validate_id("project_id", project_id, 128)?;
        validate_id("workload_id", workload_id, 128)?;
        validate_cancel_reason(request.reason.as_deref())?;
        assert_project_binding(auth, project_id)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_opt(
                &format!("{} FOR UPDATE OF w, j", customer_job_select()),
                &[&workload_id, &project_id, &auth.organization_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("customer workload not found".to_string()))?;
        let job_id: String = row.get("job_id");
        let job_status: String = row.get("job_status");
        if job_status == "cancelled" {
            let response = customer_job_response(request_id, row, true)?;
            transaction.commit().await?;
            return Ok(response);
        }
        if matches!(job_status.as_str(), "succeeded" | "failed") {
            return Err(SessionError::Conflict(
                "terminal customer jobs cannot be cancelled".to_string(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        mark_lease_terminal_for_job(
            &transaction,
            &job_id,
            "cancelled",
            request.reason.as_deref().or(Some("customer_cancelled")),
            &now,
        )
        .await?;
        let cancelled = transaction
            .execute(
                "UPDATE compute_jobs SET status = 'cancelled', cancellation_reason = $1, completed_at = $2, updated_at = $2, job_credential_hash = NULL, job_credential_expires_at = NULL WHERE job_id = $3 AND status NOT IN ('succeeded', 'failed', 'cancelled')",
                &[&request.reason, &now, &job_id],
            )
            .await?;
        if cancelled != 1 {
            return Err(SessionError::Conflict(
                "customer job cancellation lost state authority".to_string(),
            ));
        }
        release_customer_placement(&transaction, &job_id).await?;
        append_usage_ledger_for_job(&transaction, request_id, &job_id, &now).await?;
        insert_customer_cancel_audit(
            &transaction,
            auth,
            project_id,
            workload_id,
            &job_id,
            request.reason.as_deref(),
            &now,
        )
        .await?;
        let updated = transaction
            .query_one(
                customer_job_select(),
                &[&workload_id, &project_id, &auth.organization_id],
            )
            .await?;
        let response = customer_job_response(request_id, updated, false)?;
        transaction.commit().await?;
        Ok(response)
    }
}

async fn load_owned_customer_job(
    client: &tokio_postgres::Client,
    auth: &CustomerApiKeyAuth,
    project_id: &str,
    workload_id: &str,
) -> Result<Row, SessionError> {
    client
        .query_opt(
            customer_job_select(),
            &[&workload_id, &project_id, &auth.organization_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("customer workload not found".to_string()))
}

fn customer_job_select() -> &'static str {
    "SELECT w.workload_id, w.organization_id, w.project_id, w.reservation_id, w.schema_version AS workload_schema_version, w.client_workload_id, w.workload_type, w.requirements_json, w.status AS workload_status, w.job_id, w.created_at AS workload_created_at, w.updated_at AS workload_updated_at, j.status AS job_status, j.progress_percent, j.error_code, j.result_artifacts_json, j.created_at AS job_created_at, j.started_at, j.completed_at, j.updated_at AS job_updated_at FROM customer_workloads w JOIN compute_jobs j ON j.job_id = w.job_id AND j.workload_id = w.workload_id WHERE w.workload_id = $1 AND w.project_id = $2 AND w.organization_id = $3"
}

fn customer_job_response(
    request_id: &str,
    row: Row,
    duplicate: bool,
) -> Result<CustomerJobResponse, SessionError> {
    let requirements_json: String = row.get("requirements_json");
    let result_artifacts_json: String = row.get("result_artifacts_json");
    let workload = CustomerWorkloadRecord {
        workload_id: row.get("workload_id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        schema_version: row.get("workload_schema_version"),
        client_workload_id: row.get("client_workload_id"),
        reservation_id: row.get("reservation_id"),
        workload_type: row.get("workload_type"),
        requirements: serde_json::from_str(&requirements_json)
            .map_err(|error| SessionError::Invalid(error.to_string()))?,
        status: row.get("workload_status"),
        job_id: Some(row.get("job_id")),
        created_at: row.get("workload_created_at"),
        updated_at: row.get("workload_updated_at"),
    };
    let internal_artifacts: Vec<JobArtifact> = serde_json::from_str(&result_artifacts_json)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    let job = CustomerJobRecord {
        schema_version: CUSTOMER_JOB_SCHEMA_VERSION.to_string(),
        workload_id: workload.workload_id.clone(),
        job_id: workload.job_id.clone().unwrap_or_default(),
        workload_type: workload.workload_type.clone(),
        status: public_job_status(row.get::<_, String>("job_status").as_str()).to_string(),
        progress_percent: row.get("progress_percent"),
        error_code: row.get("error_code"),
        result_artifacts: internal_artifacts
            .into_iter()
            .map(public_result_artifact)
            .collect(),
        created_at: row.get("job_created_at"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        updated_at: row.get("job_updated_at"),
    };
    Ok(CustomerJobResponse {
        request_id: request_id.to_string(),
        workload,
        job,
        duplicate,
    })
}

fn public_result_artifact(artifact: JobArtifact) -> CustomerResultArtifact {
    CustomerResultArtifact {
        artifact_id: artifact.artifact_id,
        role: artifact.role,
        sha256: artifact.sha256,
        size_bytes: artifact.size_bytes,
        content_type: artifact.content_type,
    }
}

fn customer_event_from_row(row: Row) -> CustomerJobEventRecord {
    CustomerJobEventRecord {
        schema_version: CUSTOMER_JOB_EVENT_SCHEMA_VERSION.to_string(),
        event_id: row.get("event_id"),
        job_id: row.get("job_id"),
        sequence: u64::try_from(row.get::<_, i64>("sequence")).unwrap_or_default(),
        event_type: public_event_type(row.get::<_, String>("event_type").as_str()).to_string(),
        progress_percent: row.get("progress_percent"),
        occurred_at: row.get("occurred_at"),
    }
}

fn public_job_status(status: &str) -> &str {
    match status {
        "queued" | "assigned" | "accepted" => "queued",
        "provisioning" => "provisioning",
        "running" => "running",
        "uploading" => "uploading",
        "succeeded" => "succeeded",
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "failed",
    }
}

fn public_event_type(event_type: &str) -> &str {
    match event_type {
        "provisioning" => "provisioning",
        "started" | "running" => "running",
        "uploading" => "uploading",
        "progress" => "progress",
        "cleanup_completed" => "cleanup_completed",
        _ => "update",
    }
}

async fn insert_customer_cancel_audit(
    transaction: &Transaction<'_>,
    auth: &CustomerApiKeyAuth,
    project_id: &str,
    workload_id: &str,
    job_id: &str,
    reason: Option<&str>,
    now: &str,
) -> Result<(), SessionError> {
    transaction
        .execute(
            "INSERT INTO customer_audit_events (customer_audit_event_id, organization_id, project_id, schema_version, actor_type, actor_id, event_type, entity_type, entity_id, summary, metadata_json, occurred_at) VALUES ($1, $2, $3, 'burd-customer-audit-v1', 'customer_api_key', $4, 'customer_workload.cancelled', 'customer_workload', $5, 'customer workload cancelled', $6, $7)",
            &[
                &format!("customer_audit_{}", Uuid::new_v4()),
                &auth.organization_id,
                &project_id,
                &auth.api_key_id,
                &workload_id,
                &serde_json::json!({"job_id": job_id, "reason": reason}).to_string(),
                &now,
            ],
        )
        .await?;
    Ok(())
}

fn require_customer_scope(auth: &CustomerApiKeyAuth, scope: &str) -> Result<(), SessionError> {
    if auth.scopes.iter().any(|candidate| candidate == scope) {
        Ok(())
    } else {
        Err(SessionError::Unauthorized)
    }
}

fn assert_project_binding(auth: &CustomerApiKeyAuth, project_id: &str) -> Result<(), SessionError> {
    if auth
        .project_id
        .as_deref()
        .is_some_and(|bound| bound != project_id)
    {
        Err(SessionError::Unauthorized)
    } else {
        Ok(())
    }
}

fn validate_id(label: &str, value: &str, maximum_len: usize) -> Result<(), SessionError> {
    if value.is_empty()
        || value.len() > maximum_len
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return Err(SessionError::Invalid(format!("{label} is invalid")));
    }
    Ok(())
}

fn validate_cancel_reason(reason: Option<&str>) -> Result<(), SessionError> {
    if reason.is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_CANCEL_REASON_LEN
            || value.chars().any(char::is_control)
            || contains_secret_text(value)
    }) {
        return Err(SessionError::Invalid(
            "customer cancellation reason is invalid".to_string(),
        ));
    }
    Ok(())
}

fn contains_secret_text(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "password",
        "secret",
        "private_key",
        "api_key",
        "api-key",
        "authorization",
        "credential",
        "access_token",
        "access-token",
        "bearer ",
        "jobcred",
        "resume_token",
        "resume-token",
    ]
    .iter()
    .any(|needle| value.contains(needle))
        || value
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|word| word == "token")
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{ComputeRequirements, JOB_SCHEMA_VERSION};

    fn auth(suffix: &str) -> CustomerApiKeyAuth {
        CustomerApiKeyAuth {
            api_key_id: format!("api_key_{suffix}"),
            organization_id: format!("org_{suffix}"),
            project_id: Some(format!("project_{suffix}")),
            scopes: vec!["workloads:read".to_string(), "workloads:write".to_string()],
        }
    }

    async fn seed_customer_job(
        client: &tokio_postgres::Client,
        suffix: &str,
        status: &str,
        now: &str,
    ) {
        let provider_id = format!("provider_{suffix}");
        let device_id = format!("device_{suffix}");
        let session_id = format!("session_{suffix}");
        let workload_id = format!("workload_{suffix}");
        let placement_id = format!("placement_{suffix}");
        let job_id = format!("job_{suffix}");
        let listing_id = format!("listing_{suffix}");
        client.execute("INSERT INTO providers (provider_id, status, created_at, updated_at) VALUES ($1, 'available', $2, $2)", &[&provider_id, &now]).await.unwrap();
        client.execute("INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ($1, $2, $3, 'active', $4, $4)", &[&device_id, &provider_id, &format!("machine_{suffix}"), &now]).await.unwrap();
        client.execute("INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint) VALUES ($1, $2, $3, 'online', 0, $4, $5, $6)", &[&session_id, &provider_id, &device_id, &now, &"2099-01-01T00:00:00Z", &"a".repeat(64)]).await.unwrap();
        client.execute("INSERT INTO workload_policies (policy_id, policy_version, schema_version, workload_type, display_name, requirements_json, status, created_at, updated_at) VALUES ('policy_1', 'v1', 'burd-workload-policy-v1', 'llm_realtime_api', 'Policy', '{}', 'active', $1, $1)", &[&now]).await.unwrap();
        client.execute("INSERT INTO organizations (organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ($1, 'burd-customer-organization-v1', 'Org', 'active', $2, $2)", &[&format!("org_{suffix}"), &now]).await.unwrap();
        client.execute("INSERT INTO projects (project_id, organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ($1, $2, 'burd-customer-project-v1', 'Project', 'active', $3, $3)", &[&format!("project_{suffix}"), &format!("org_{suffix}"), &now]).await.unwrap();
        client.execute("INSERT INTO marketplace_listings (listing_id, provider_id, device_id, session_id, schema_version, engine_version, status, current_status, workload_type, policy_id, policy_version, gpu_uuid, gpu_verified, gpu_verification_source, vram_total_mib, vram_verified, vram_verification_source, region, region_source, trust_score, risk_score, reliability_score, proof_freshness_status, price_currency, price_per_hour_micros, price_source, availability_window_json, active_lease_count, reason_codes_json, source_hash, published_at, updated_at) VALUES ($1, $2, $3, $4, 'burd-marketplace-listing-v1', 'burd-marketplace-engine-v1', 'published', 'available', 'llm_realtime_api', 'policy_1', 'v1', $5, TRUE, 'backend', 24576, TRUE, 'backend', 'br-southeast', 'backend', 90, 10, 99, 'fresh', 'BRL', 1000000, 'configured', '{}', 0, '[]', $6, $7, $7)", &[&listing_id, &provider_id, &device_id, &session_id, &format!("GPU-{suffix}"), &format!("source_{suffix}"), &now]).await.unwrap();
        let requirements = serde_json::to_string(&ComputeRequirements {
            gpu_count: 1,
            backend: "cuda".to_string(),
            minimum_vram_mib: None,
            region: None,
            minimum_trust_score: None,
            maximum_risk_score: None,
            minimum_reliability_score: None,
            maximum_price_per_hour_micros: None,
        })
        .unwrap();
        client.execute("INSERT INTO customer_workloads (workload_id, organization_id, project_id, schema_version, workload_type, requirements_json, parameters_json, timeout_seconds, status, idempotency_key, request_hash, created_at, updated_at) VALUES ($1, $2, $3, 'burd-customer-workload-v1', 'llm_realtime_api', $4, '{}', 900, 'queued', $5, $6, $7, $7)", &[&workload_id, &format!("org_{suffix}"), &format!("project_{suffix}"), &requirements, &format!("idem_{suffix}"), &format!("hash_{suffix}"), &now]).await.unwrap();
        client.execute("INSERT INTO compute_placements (placement_id, workload_id, schema_version, listing_id, provider_id, device_id, session_id, gpu_uuid, policy_id, policy_version, status, reason_codes_json, runtime_admission_json, created_at) VALUES ($1, $2, 'burd-placement-v1', $3, $4, $5, $6, $7, 'policy_1', 'v1', 'selected', '[]', '{}', $8)", &[&placement_id, &workload_id, &listing_id, &provider_id, &device_id, &session_id, &format!("GPU-{suffix}"), &now]).await.unwrap();
        client.execute("INSERT INTO compute_jobs (job_id, provider_id, device_id, session_id, schema_version, workload_type, template_id, image_ref, gpu_uuid, backend, parameters_json, input_artifacts_json, expected_outputs_json, result_artifacts_json, result_metrics_json, status, timeout_seconds, workload_id, placement_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 'llm_realtime_api', 'llm_inference', $6, $7, 'cuda', '{}', '[]', '[]', $8, '{}', 'queued', 900, $9, $10, $11, $11)", &[&job_id, &provider_id, &device_id, &session_id, &JOB_SCHEMA_VERSION, &format!("ghcr.io/burd/runtime@sha256:{}", "a".repeat(64)), &format!("GPU-{suffix}"), &serde_json::json!([{"artifact_id":"result_1","role":"output","object_key":"private/result","sha256":format!("sha256:{}", "b".repeat(64)),"size_bytes":12}]).to_string(), &workload_id, &placement_id, &now]).await.unwrap();
        if matches!(
            status,
            "assigned" | "accepted" | "provisioning" | "running" | "uploading"
        ) {
            let lease_id = format!("lease_{suffix}");
            let lease_status = match status {
                "assigned" => "offered",
                "accepted" => "accepted",
                "provisioning" => "provisioning",
                _ => "active",
            };
            client.execute("INSERT INTO job_leases (lease_id, job_id, provider_id, device_id, session_id, schema_version, workload_type, gpu_uuid, status, reason_codes_json, offered_at, expires_at, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 'burd-job-lease-v1', 'llm_realtime_api', $6, $7, '[]', $8, '2099-01-01T00:00:00Z', $8, $8)", &[&lease_id, &job_id, &provider_id, &device_id, &session_id, &format!("GPU-{suffix}"), &lease_status, &now]).await.unwrap();
            client.execute("UPDATE compute_jobs SET status = $1, assignment_lease_id = $2 WHERE job_id = $3", &[&status, &lease_id, &job_id]).await.unwrap();
        } else {
            client
                .execute(
                    "UPDATE compute_jobs SET status = $1 WHERE job_id = $2",
                    &[&status, &job_id],
                )
                .await
                .unwrap();
        }
        client
            .execute(
                "UPDATE customer_workloads SET status = 'placed', job_id = $1 WHERE workload_id = $2",
                &[&job_id, &workload_id],
            )
            .await
            .unwrap();
    }

    #[test]
    fn public_state_projection_collapses_internal_assignment_states() {
        for internal in ["queued", "assigned", "accepted"] {
            assert_eq!(public_job_status(internal), "queued");
        }
        assert_eq!(public_job_status("running"), "running");
        assert_eq!(public_event_type("log"), "update");
    }

    #[test]
    fn cancellation_reason_rejects_secret_like_text() {
        assert!(validate_cancel_reason(Some("customer request")).is_ok());
        assert!(validate_cancel_reason(Some("authorization token leaked")).is_err());
        assert!(validate_cancel_reason(Some("bearer abc123")).is_err());
        assert!(validate_cancel_reason(Some("api-key leaked")).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_customer_projection_redacts_internal_authority_and_events() {
        let db =
            crate::scheduler::tests::postgres_test_database("burd_customer_job_projection").await;
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        seed_customer_job(&client, "projection", "running", &now).await;
        client.execute("INSERT INTO job_events (event_id, job_id, provider_id, device_id, session_id, sequence, schema_version, event_type, progress_percent, message, metadata_json, occurred_at, server_received_at) VALUES ('event_projection', 'job_projection', 'provider_projection', 'device_projection', 'session_projection', 1, 'burd-job-event-v1', 'running', 25, 'running', '{\"private\":\"metadata\"}', $1, $1)", &[&now]).await.unwrap();

        let response = db
            .get_customer_job(
                "req",
                &auth("projection"),
                "project_projection",
                "workload_projection",
            )
            .await
            .unwrap();
        let serialized = serde_json::to_string(&response).unwrap();
        for private in [
            "provider_id",
            "device_id",
            "session_id",
            "gpu_uuid",
            "object_key",
            "private/result",
        ] {
            assert!(!serialized.contains(private));
        }
        assert_eq!(response.job.status, "running");
        assert_eq!(response.job.result_artifacts[0].artifact_id, "result_1");
        let events = db
            .list_customer_job_events(
                "req",
                &auth("projection"),
                "project_projection",
                "workload_projection",
                20,
            )
            .await
            .unwrap();
        let events_json = serde_json::to_string(&events).unwrap();
        assert!(!events_json.contains("metadata"));
        assert!(!events_json.contains("message"));
        assert_eq!(events.events[0].event_type, "running");
        assert!(
            db.get_customer_job(
                "req",
                &auth("other"),
                "project_other",
                "workload_projection"
            )
            .await
            .is_err()
        );
        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_customer_cancel_is_idempotent_and_terminal_safe() {
        let db = crate::scheduler::tests::postgres_test_database("burd_customer_job_cancel").await;
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        seed_customer_job(&client, "cancel", "running", &now).await;
        let request = CancelCustomerWorkloadRequest {
            reason: Some("customer request".to_string()),
        };
        let first_auth = auth("cancel");
        let second_auth = auth("cancel");
        let (first, duplicate) = tokio::join!(
            db.cancel_customer_job(
                "req_1",
                &first_auth,
                "project_cancel",
                "workload_cancel",
                &request,
            ),
            db.cancel_customer_job(
                "req_2",
                &second_auth,
                "project_cancel",
                "workload_cancel",
                &request,
            ),
        );
        let first = first.unwrap();
        let duplicate = duplicate.unwrap();
        assert_eq!(first.job.status, "cancelled");
        assert_eq!(duplicate.job.status, "cancelled");
        assert_ne!(first.duplicate, duplicate.duplicate);
        let state = client.query_one("SELECT j.status AS job_status, w.status AS workload_status, p.status AS placement_status, l.status AS lease_status, j.job_credential_hash FROM compute_jobs j JOIN customer_workloads w ON w.job_id = j.job_id JOIN compute_placements p ON p.placement_id = j.placement_id JOIN job_leases l ON l.job_id = j.job_id WHERE j.job_id = 'job_cancel'", &[]).await.unwrap();
        assert_eq!(state.get::<_, String>("job_status"), "cancelled");
        assert_eq!(state.get::<_, String>("workload_status"), "cancelled");
        assert_eq!(state.get::<_, String>("placement_status"), "released");
        assert_eq!(state.get::<_, String>("lease_status"), "failed");
        assert_eq!(state.get::<_, Option<String>>("job_credential_hash"), None);
        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_customer_cancels_queued_job_before_any_lease() {
        let db = crate::scheduler::tests::postgres_test_database("burd_customer_job_queued_cancel")
            .await;
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        seed_customer_job(&client, "queued_cancel", "queued", &now).await;
        let cancelled = db
            .cancel_customer_job(
                "req",
                &auth("queued_cancel"),
                "project_queued_cancel",
                "workload_queued_cancel",
                &CancelCustomerWorkloadRequest { reason: None },
            )
            .await
            .unwrap();
        assert_eq!(cancelled.job.status, "cancelled");
        let lease_count: i64 = client
            .query_one(
                "SELECT COUNT(*) AS count FROM job_leases WHERE job_id = 'job_queued_cancel'",
                &[],
            )
            .await
            .unwrap()
            .get("count");
        assert_eq!(lease_count, 0);
        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_customer_cannot_cancel_succeeded_job() {
        let db =
            crate::scheduler::tests::postgres_test_database("burd_customer_job_terminal").await;
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        seed_customer_job(&client, "terminal", "succeeded", &now).await;
        let request = CancelCustomerWorkloadRequest { reason: None };
        assert!(
            db.cancel_customer_job(
                "req",
                &auth("terminal"),
                "project_terminal",
                "workload_terminal",
                &request
            )
            .await
            .is_err()
        );
        let status: String = client
            .query_one(
                "SELECT status FROM compute_jobs WHERE job_id = 'job_terminal'",
                &[],
            )
            .await
            .unwrap()
            .get("status");
        assert_eq!(status, "succeeded");
        let mut transaction_client = db.connect().await.unwrap();
        let transaction = transaction_client.transaction().await.unwrap();
        release_customer_placement(&transaction, "job_terminal")
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let workload_status: String = client
            .query_one(
                "SELECT status FROM customer_workloads WHERE workload_id = 'workload_terminal'",
                &[],
            )
            .await
            .unwrap()
            .get("status");
        assert_eq!(workload_status, "succeeded");
        db.drop_schema_for_test().await.unwrap();
    }
}
