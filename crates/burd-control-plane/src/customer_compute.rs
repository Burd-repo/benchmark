use crate::db::{Database, DbError, IdempotencyRecord};
use crate::remote_session::SessionError;
use crate::runtime_admission::{
    RuntimeAdmissionPolicy, evaluate_runtime_admission_for_gpu_in_transaction,
};
use burd_protocol::{
    CUSTOMER_WORKLOAD_SCHEMA_VERSION, ComputeRequirements, CreateCustomerWorkloadRequest,
    CustomerWorkloadRecord, CustomerWorkloadResponse, JOB_SCHEMA_VERSION, PLACEMENT_SCHEMA_VERSION,
};
use chrono::Utc;
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const DEFAULT_WORKLOAD_TIMEOUT_SECONDS: u32 = 3_600;
const MAX_WORKLOAD_TIMEOUT_SECONDS: u32 = 24 * 60 * 60;

#[derive(Debug, Clone)]
pub struct CreateCustomerWorkloadCommand {
    pub request_id: String,
    pub scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub auth: crate::customer::CustomerApiKeyAuth,
    pub project_id: String,
    pub request: CreateCustomerWorkloadRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateCustomerWorkloadOutcome {
    Response(IdempotencyRecord),
    Conflict,
}

#[derive(Debug)]
struct ProjectAccess {
    organization_id: String,
    project_id: String,
}

#[derive(Debug)]
struct PlacementCandidate {
    listing_id: String,
    provider_id: String,
    device_id: String,
    session_id: String,
    gpu_uuid: String,
    policy_id: String,
    policy_version: String,
}

struct CustomerWorkloadAudit<'a> {
    api_key_id: &'a str,
    workload_id: &'a str,
    placement_id: &'a str,
    job_id: &'a str,
    listing_id: &'a str,
    occurred_at: &'a str,
}

impl Database {
    pub async fn create_customer_workload_idempotently(
        &self,
        command: CreateCustomerWorkloadCommand,
        runtime_admission_policy: &RuntimeAdmissionPolicy,
    ) -> Result<CreateCustomerWorkloadOutcome, SessionError> {
        require_customer_scope(&command.auth, "workloads:write")?;
        validate_id("project_id", &command.project_id, 128)?;
        validate_workload_request(&command.request)?;

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let reserved = transaction
            .execute(
                "INSERT INTO idempotency_keys (scope, idempotency_key, request_hash, status_code, response_json, created_at) VALUES ($1, $2, $3, 0, '', $4) ON CONFLICT (scope, idempotency_key) DO NOTHING",
                &[&command.scope, &command.idempotency_key, &command.request_hash, &now_text],
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
                Ok(CreateCustomerWorkloadOutcome::Response(record))
            } else {
                Ok(CreateCustomerWorkloadOutcome::Conflict)
            };
        }

        let project =
            authorize_project_access(&transaction, &command.auth, &command.project_id).await?;
        let candidate = select_placement_candidate(
            &transaction,
            &command.request.workload_type,
            &command.request.requirements,
        )
        .await?;
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
            return Err(SessionError::Conflict(
                "no runtime-admitted marketplace supply satisfies the workload".to_string(),
            ));
        }
        if gpu_has_selected_placement(
            &transaction,
            &candidate.provider_id,
            &candidate.device_id,
            &candidate.gpu_uuid,
        )
        .await?
        {
            return Err(SessionError::Conflict(
                "marketplace supply is already placed".to_string(),
            ));
        }

        let workload_id = format!("workload_{}", Uuid::new_v4());
        let placement_id = format!("placement_{}", Uuid::new_v4());
        let job_id = format!("job_{}", Uuid::new_v4());
        let timeout_seconds = command
            .request
            .timeout_seconds
            .unwrap_or(DEFAULT_WORKLOAD_TIMEOUT_SECONDS);
        let requirements_json = serde_json::to_string(&command.request.requirements)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let parameters_json = normalized_json_object(&command.request.parameters)?;
        transaction
            .execute(
                "INSERT INTO customer_workloads (workload_id, organization_id, project_id, schema_version, client_workload_id, workload_type, requirements_json, parameters_json, timeout_seconds, status, idempotency_key, request_hash, job_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'queued', $10, $11, NULL, $12, $12)",
                &[
                    &workload_id,
                    &project.organization_id,
                    &project.project_id,
                    &CUSTOMER_WORKLOAD_SCHEMA_VERSION,
                    &command.request.client_workload_id,
                    &command.request.workload_type,
                    &requirements_json,
                    &parameters_json,
                    &(timeout_seconds as i32),
                    &command.idempotency_key,
                    &command.request_hash,
                    &now_text,
                ],
            )
            .await?;
        let reason_codes = vec![
            "marketplace_listing_available".to_string(),
            "workload_requirements_satisfied".to_string(),
            "runtime_admission_admitted".to_string(),
        ];
        let reason_codes_json = serde_json::to_string(&reason_codes)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let admission_json = serde_json::to_string(&admission)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO compute_placements (placement_id, workload_id, schema_version, listing_id, provider_id, device_id, session_id, gpu_uuid, policy_id, policy_version, status, reason_codes_json, runtime_admission_json, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'selected', $11, $12, $13)",
                &[
                    &placement_id,
                    &workload_id,
                    &PLACEMENT_SCHEMA_VERSION,
                    &candidate.listing_id,
                    &candidate.provider_id,
                    &candidate.device_id,
                    &candidate.session_id,
                    &candidate.gpu_uuid,
                    &candidate.policy_id,
                    &candidate.policy_version,
                    &reason_codes_json,
                    &admission_json,
                    &now_text,
                ],
            )
            .await?;
        let (template_id, image_ref) =
            load_backend_execution_contract(&transaction, &command.request.workload_type).await?;
        transaction
            .execute(
                "INSERT INTO compute_jobs (job_id, client_job_id, provider_id, device_id, session_id, schema_version, workload_type, template_id, image_ref, gpu_uuid, backend, parameters_json, input_artifacts_json, expected_outputs_json, result_artifacts_json, result_metrics_json, policy_id, policy_version, status, timeout_seconds, workload_id, placement_id, created_at, updated_at) VALUES ($1, NULL, $2, $3, $4, $5, $6, $7, $8, $9, 'cuda', $10, '[]', '[]', '[]', '{}', $11, $12, 'queued', $13, $14, $15, $16, $16)",
                &[
                    &job_id,
                    &candidate.provider_id,
                    &candidate.device_id,
                    &candidate.session_id,
                    &JOB_SCHEMA_VERSION,
                    &command.request.workload_type,
                    &template_id,
                    &image_ref,
                    &candidate.gpu_uuid,
                    &parameters_json,
                    &candidate.policy_id,
                    &candidate.policy_version,
                    &(timeout_seconds as i32),
                    &workload_id,
                    &placement_id,
                    &now_text,
                ],
            )
            .await?;
        transaction
            .execute(
                "UPDATE customer_workloads SET status = 'placed', job_id = $1, updated_at = $2 WHERE workload_id = $3 AND status = 'queued'",
                &[&job_id, &now_text, &workload_id],
            )
            .await?;
        let workload = load_workload(&transaction, &workload_id).await?;
        insert_customer_audit_event(
            &transaction,
            &project,
            CustomerWorkloadAudit {
                api_key_id: &command.auth.api_key_id,
                workload_id: &workload_id,
                placement_id: &placement_id,
                job_id: &job_id,
                listing_id: &candidate.listing_id,
                occurred_at: &now_text,
            },
        )
        .await?;
        let response_json = serde_json::to_string(&CustomerWorkloadResponse {
            request_id: command.request_id,
            workload,
            duplicate: false,
        })
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
        let status_code = 201_i32;
        transaction
            .execute(
                "UPDATE idempotency_keys SET status_code = $1, response_json = $2 WHERE scope = $3 AND idempotency_key = $4",
                &[&status_code, &response_json, &command.scope, &command.idempotency_key],
            )
            .await?;
        transaction.commit().await?;
        Ok(CreateCustomerWorkloadOutcome::Response(IdempotencyRecord {
            request_hash: command.request_hash,
            status_code: status_code as u16,
            response_json,
        }))
    }
}

async fn authorize_project_access(
    transaction: &Transaction<'_>,
    auth: &crate::customer::CustomerApiKeyAuth,
    project_id: &str,
) -> Result<ProjectAccess, SessionError> {
    if auth
        .project_id
        .as_deref()
        .is_some_and(|bound| bound != project_id)
    {
        return Err(SessionError::Unauthorized);
    }
    let row = transaction
        .query_opt(
            "SELECT p.project_id, p.organization_id, p.status AS project_status, o.status AS organization_status FROM projects p JOIN organizations o ON o.organization_id = p.organization_id WHERE p.project_id = $1 FOR UPDATE OF p",
            &[&project_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("project not found".to_string()))?;
    let organization_id: String = row.get("organization_id");
    if organization_id != auth.organization_id {
        return Err(SessionError::Unauthorized);
    }
    if row.get::<_, String>("project_status") != "active"
        || row.get::<_, String>("organization_status") != "active"
    {
        return Err(SessionError::Conflict(
            "project or organization is not active".to_string(),
        ));
    }
    Ok(ProjectAccess {
        organization_id,
        project_id: row.get("project_id"),
    })
}

async fn select_placement_candidate(
    transaction: &Transaction<'_>,
    workload_type: &str,
    requirements: &ComputeRequirements,
) -> Result<PlacementCandidate, SessionError> {
    let minimum_vram_mib = requirements.minimum_vram_mib.map(to_i64).transpose()?;
    let maximum_price = requirements
        .maximum_price_per_hour_micros
        .map(to_i64)
        .transpose()?;
    let row = transaction
        .query_opt(
            "SELECT l.listing_id, l.provider_id, l.device_id, l.session_id, l.gpu_uuid, l.policy_id, l.policy_version FROM marketplace_listings l WHERE l.workload_type = $1 AND l.status IN ('published', 'limited') AND l.current_status = 'available' AND l.gpu_verified = TRUE AND l.vram_verified = TRUE AND l.session_id IS NOT NULL AND l.gpu_uuid IS NOT NULL AND ($2::BIGINT IS NULL OR l.vram_total_mib >= $2) AND ($3::TEXT IS NULL OR l.region = $3) AND ($4::DOUBLE PRECISION IS NULL OR l.trust_score >= $4) AND ($5::DOUBLE PRECISION IS NULL OR l.risk_score <= $5) AND ($6::DOUBLE PRECISION IS NULL OR l.reliability_score >= $6) AND ($7::BIGINT IS NULL OR (l.price_per_hour_micros IS NOT NULL AND l.price_per_hour_micros <= $7)) AND NOT EXISTS (SELECT 1 FROM job_leases jl WHERE jl.provider_id = l.provider_id AND jl.device_id = l.device_id AND lower(jl.gpu_uuid) = lower(l.gpu_uuid) AND jl.status IN ('offered', 'accepted', 'provisioning', 'active')) ORDER BY l.price_per_hour_micros ASC NULLS LAST, l.trust_score DESC NULLS LAST, l.reliability_score DESC NULLS LAST, l.updated_at DESC, l.listing_id FOR UPDATE OF l SKIP LOCKED LIMIT 1",
            &[
                &workload_type,
                &minimum_vram_mib,
                &requirements.region,
                &requirements.minimum_trust_score,
                &requirements.maximum_risk_score,
                &requirements.minimum_reliability_score,
                &maximum_price,
            ],
        )
        .await?
        .ok_or_else(|| SessionError::Conflict("no marketplace supply satisfies the workload".to_string()))?;
    Ok(PlacementCandidate {
        listing_id: row.get("listing_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        gpu_uuid: row.get("gpu_uuid"),
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
    })
}

async fn load_backend_execution_contract(
    transaction: &Transaction<'_>,
    workload_type: &str,
) -> Result<(String, String), SessionError> {
    let row = transaction
        .query_opt(
            "SELECT template_id, image_ref FROM workload_execution_profiles WHERE workload_type = $1 AND status = 'active'",
            &[&workload_type],
        )
        .await?
        .ok_or_else(|| SessionError::Conflict("workload has no active execution profile".to_string()))?;
    Ok((row.get("template_id"), row.get("image_ref")))
}

async fn gpu_has_selected_placement(
    transaction: &Transaction<'_>,
    provider_id: &str,
    device_id: &str,
    gpu_uuid: &str,
) -> Result<bool, SessionError> {
    Ok(transaction
        .query_opt(
            "SELECT placement_id FROM compute_placements WHERE provider_id = $1 AND device_id = $2 AND lower(gpu_uuid) = lower($3) AND status = 'selected' LIMIT 1",
            &[&provider_id, &device_id, &gpu_uuid],
        )
        .await?
        .is_some())
}

async fn load_workload(
    transaction: &Transaction<'_>,
    workload_id: &str,
) -> Result<CustomerWorkloadRecord, SessionError> {
    let row = transaction
        .query_one(
            "SELECT workload_id, organization_id, project_id, schema_version, client_workload_id, workload_type, requirements_json, status, job_id, created_at, updated_at FROM customer_workloads WHERE workload_id = $1",
            &[&workload_id],
        )
        .await?;
    let requirements_json: String = row.get("requirements_json");
    Ok(CustomerWorkloadRecord {
        workload_id: row.get("workload_id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        schema_version: row.get("schema_version"),
        client_workload_id: row.get("client_workload_id"),
        workload_type: row.get("workload_type"),
        requirements: serde_json::from_str(&requirements_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        status: row.get("status"),
        job_id: row.get("job_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

async fn insert_customer_audit_event(
    transaction: &Transaction<'_>,
    project: &ProjectAccess,
    audit: CustomerWorkloadAudit<'_>,
) -> Result<(), SessionError> {
    let metadata = serde_json::json!({
        "placement_id": audit.placement_id,
        "job_id": audit.job_id,
        "listing_id": audit.listing_id,
    })
    .to_string();
    transaction
        .execute(
            "INSERT INTO customer_audit_events (customer_audit_event_id, organization_id, project_id, schema_version, actor_type, actor_id, event_type, entity_type, entity_id, summary, metadata_json, occurred_at) VALUES ($1, $2, $3, 'burd-customer-audit-v1', 'customer_api_key', $4, 'customer_workload.placed', 'customer_workload', $5, 'customer workload placed and queued', $6, $7)",
            &[
                &format!("customer_audit_{}", Uuid::new_v4()),
                &project.organization_id,
                &project.project_id,
                &audit.api_key_id,
                &audit.workload_id,
                &metadata,
                &audit.occurred_at,
            ],
        )
        .await?;
    Ok(())
}

fn validate_workload_request(request: &CreateCustomerWorkloadRequest) -> Result<(), SessionError> {
    if let Some(client_workload_id) = request.client_workload_id.as_deref() {
        validate_id("client_workload_id", client_workload_id, 128)?;
    }
    validate_id("workload_type", &request.workload_type, 96)?;
    if request.requirements.gpu_count != 1 {
        return Err(SessionError::Invalid(
            "single-GPU v1 requires gpu_count equal to 1".to_string(),
        ));
    }
    if request.requirements.backend != "cuda" {
        return Err(SessionError::Invalid(
            "single-GPU v1 requires cuda backend".to_string(),
        ));
    }
    if request.requirements.minimum_vram_mib == Some(0) {
        return Err(SessionError::Invalid(
            "minimum_vram_mib must be greater than zero".to_string(),
        ));
    }
    if request.requirements.maximum_price_per_hour_micros == Some(0) {
        return Err(SessionError::Invalid(
            "maximum_price_per_hour_micros must be greater than zero".to_string(),
        ));
    }
    if let Some(region) = request.requirements.region.as_deref() {
        validate_id("region", region, 64)?;
    }
    for (label, value) in [
        (
            "minimum_trust_score",
            request.requirements.minimum_trust_score,
        ),
        (
            "maximum_risk_score",
            request.requirements.maximum_risk_score,
        ),
        (
            "minimum_reliability_score",
            request.requirements.minimum_reliability_score,
        ),
    ] {
        if value.is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value)) {
            return Err(SessionError::Invalid(format!(
                "{label} must be between 0 and 100"
            )));
        }
    }
    if let Some(timeout) = request.timeout_seconds
        && (timeout == 0 || timeout > MAX_WORKLOAD_TIMEOUT_SECONDS)
    {
        return Err(SessionError::Invalid(
            "timeout_seconds is outside allowed range".to_string(),
        ));
    }
    normalized_json_object(&request.parameters)?;
    Ok(())
}

fn normalized_json_object(value: &serde_json::Value) -> Result<String, SessionError> {
    if !value.is_object() {
        return Err(SessionError::Invalid(
            "workload parameters must be a JSON object".to_string(),
        ));
    }
    let json =
        serde_json::to_string(value).map_err(|error| SessionError::Invalid(error.to_string()))?;
    if json.len() > 64 * 1024 || contains_secret_field(value) {
        return Err(SessionError::Invalid(
            "workload parameters are too large or contain secret-like fields".to_string(),
        ));
    }
    Ok(json)
}

fn contains_secret_field(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            let key = key.to_ascii_lowercase();
            matches!(
                key.as_str(),
                "authorization" | "password" | "private_key" | "secret" | "token"
            ) || contains_secret_field(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_secret_field),
        _ => false,
    }
}

fn require_customer_scope(
    auth: &crate::customer::CustomerApiKeyAuth,
    scope: &str,
) -> Result<(), SessionError> {
    if auth.scopes.iter().any(|value| value == scope) {
        Ok(())
    } else {
        Err(SessionError::Unauthorized)
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

fn to_i64(value: u64) -> Result<i64, SessionError> {
    i64::try_from(value)
        .map_err(|_| SessionError::Invalid("numeric value is too large".to_string()))
}

fn idempotency_from_row(row: Row) -> IdempotencyRecord {
    IdempotencyRecord {
        request_hash: row.get("request_hash"),
        status_code: row.get::<_, i32>("status_code") as u16,
        response_json: row.get("response_json"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{COMPUTE_REQUIREMENTS_SCHEMA_VERSION, PROVIDER_JOB_APPROVED_TEMPLATES};

    fn request() -> CreateCustomerWorkloadRequest {
        CreateCustomerWorkloadRequest {
            client_workload_id: Some("request-1".to_string()),
            workload_type: "llm_realtime_api".to_string(),
            requirements: ComputeRequirements {
                gpu_count: 1,
                backend: "cuda".to_string(),
                minimum_vram_mib: Some(16_384),
                region: None,
                minimum_trust_score: None,
                maximum_risk_score: None,
                minimum_reliability_score: None,
                maximum_price_per_hour_micros: None,
            },
            parameters: serde_json::json!({}),
            timeout_seconds: Some(900),
        }
    }

    #[test]
    fn validates_single_gpu_cuda_requirements() {
        validate_workload_request(&request()).unwrap();
        let mut invalid = request();
        invalid.requirements.gpu_count = 2;
        assert!(validate_workload_request(&invalid).is_err());
        invalid = request();
        invalid.requirements.backend = "vulkan".to_string();
        assert!(validate_workload_request(&invalid).is_err());
    }

    #[test]
    fn workload_parameters_reject_secrets() {
        let mut invalid = request();
        invalid.parameters = serde_json::json!({"nested": {"token": "secret"}});
        assert!(validate_workload_request(&invalid).is_err());
    }

    #[test]
    fn approved_execution_templates_remain_protocol_owned() {
        assert!(PROVIDER_JOB_APPROVED_TEMPLATES.contains(&"llm_inference"));
        assert_eq!(
            COMPUTE_REQUIREMENTS_SCHEMA_VERSION,
            "burd-compute-requirements-v1"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_places_customer_workload_and_queues_backend_directed_job() {
        let db = crate::scheduler::tests::postgres_test_database("burd_customer_compute").await;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let expires_at = (now + chrono::Duration::hours(1)).to_rfc3339();
        let client = db.connect().await.unwrap();
        crate::scheduler::tests::seed_provider_and_policy(&client, "provider_workload", &now_text)
            .await;
        crate::scheduler::tests::seed_device(
            &client,
            "provider_workload",
            "device_workload",
            "session_workload",
            &now_text,
            &expires_at,
        )
        .await;
        crate::scheduler::tests::seed_admitted_runtime(
            &client,
            "provider_workload",
            "device_workload",
            "session_workload",
            &["GPU-workload".to_string()],
            "GPU-workload",
            now,
        )
        .await;
        client
            .execute(
                "INSERT INTO organizations (organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ('org_workload', 'burd-customer-organization-v1', 'Org', 'active', $1, $1)",
                &[&now_text],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO projects (project_id, organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ('project_workload', 'org_workload', 'burd-customer-project-v1', 'Project', 'active', $1, $1)",
                &[&now_text],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO marketplace_listings (listing_id, provider_id, device_id, session_id, schema_version, engine_version, status, current_status, workload_type, policy_id, policy_version, gpu_uuid, gpu_verified, gpu_verification_source, vram_total_mib, vram_verified, vram_verification_source, region, region_source, trust_score, risk_score, reliability_score, proof_freshness_status, price_currency, price_per_hour_micros, price_source, availability_window_json, active_lease_count, reason_codes_json, source_hash, published_at, updated_at) VALUES ('listing_workload', 'provider_workload', 'device_workload', 'session_workload', 'burd-marketplace-listing-v1', 'burd-marketplace-engine-v1', 'published', 'available', 'llm_realtime_api', 'llm_realtime_api_cuda', '2026.07.0', 'GPU-workload', TRUE, 'backend_proof_and_benchmark', 24576, TRUE, 'backend_telemetry_bound_to_verified_gpu', 'br-southeast', 'regional_probe', 90, 10, 99, 'freshness_backend_timestamp_present', 'BRL', 1000000, 'configured', '{}', 0, '[]', 'source_workload', $1, $1)",
                &[&now_text],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO workload_execution_profiles (workload_type, template_id, image_ref, status, created_at, updated_at) VALUES ('llm_realtime_api', 'llm_inference', $1, 'active', $2, $2)",
                &[
                    &format!(
                        "ghcr.io/burd/runtime/llm@sha256:{}",
                        "c".repeat(64)
                    ),
                    &now_text,
                ],
            )
            .await
            .unwrap();

        let auth = crate::customer::CustomerApiKeyAuth {
            api_key_id: "api_key_workload".to_string(),
            organization_id: "org_workload".to_string(),
            project_id: Some("project_workload".to_string()),
            scopes: vec!["workloads:write".to_string()],
        };
        let workload_request = request();
        let request_hash = burd_protocol::hash_canonical(&workload_request).unwrap();
        let outcome = db
            .create_customer_workload_idempotently(
                CreateCustomerWorkloadCommand {
                    request_id: "req_workload".to_string(),
                    scope: "POST /v1/customer/projects/project_workload/workloads".to_string(),
                    idempotency_key: "workload-key".to_string(),
                    request_hash,
                    auth,
                    project_id: "project_workload".to_string(),
                    request: workload_request,
                },
                &crate::scheduler::tests::runtime_policy(),
            )
            .await
            .unwrap();
        let CreateCustomerWorkloadOutcome::Response(record) = outcome else {
            panic!("expected workload response");
        };
        let response: CustomerWorkloadResponse =
            serde_json::from_str(&record.response_json).unwrap();
        assert_eq!(response.workload.status, "placed");
        let job_id = response.workload.job_id.unwrap();
        let row = client
            .query_one(
                "SELECT provider_id, device_id, session_id, gpu_uuid, workload_id, placement_id, status FROM compute_jobs WHERE job_id = $1",
                &[&job_id],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>("provider_id"), "provider_workload");
        assert_eq!(row.get::<_, String>("device_id"), "device_workload");
        assert_eq!(row.get::<_, String>("session_id"), "session_workload");
        assert_eq!(row.get::<_, String>("gpu_uuid"), "GPU-workload");
        assert_eq!(
            row.get::<_, String>("workload_id"),
            response.workload.workload_id
        );
        assert!(
            row.get::<_, String>("placement_id")
                .starts_with("placement_")
        );
        assert_eq!(row.get::<_, String>("status"), "queued");
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_workload_idempotency_replays_and_rejects_conflicting_payload() {
        let db =
            crate::scheduler::tests::postgres_test_database("burd_customer_compute_idem").await;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let expires_at = (now + chrono::Duration::hours(1)).to_rfc3339();
        let client = db.connect().await.unwrap();
        crate::scheduler::tests::seed_provider_and_policy(&client, "provider_idem", &now_text)
            .await;
        crate::scheduler::tests::seed_device(
            &client,
            "provider_idem",
            "device_idem",
            "session_idem",
            &now_text,
            &expires_at,
        )
        .await;
        crate::scheduler::tests::seed_admitted_runtime(
            &client,
            "provider_idem",
            "device_idem",
            "session_idem",
            &["GPU-idem".to_string()],
            "GPU-idem",
            now,
        )
        .await;
        client
            .execute(
                "INSERT INTO organizations (organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ('org_idem', 'burd-customer-organization-v1', 'Org', 'active', $1, $1)",
                &[&now_text],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO projects (project_id, organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ('project_idem', 'org_idem', 'burd-customer-project-v1', 'Project', 'active', $1, $1)",
                &[&now_text],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO marketplace_listings (listing_id, provider_id, device_id, session_id, schema_version, engine_version, status, current_status, workload_type, policy_id, policy_version, gpu_uuid, gpu_verified, gpu_verification_source, vram_total_mib, vram_verified, vram_verification_source, region_source, trust_score, risk_score, reliability_score, proof_freshness_status, price_source, availability_window_json, active_lease_count, reason_codes_json, source_hash, published_at, updated_at) VALUES ('listing_idem', 'provider_idem', 'device_idem', 'session_idem', 'burd-marketplace-listing-v1', 'burd-marketplace-engine-v1', 'published', 'available', 'llm_realtime_api', 'llm_realtime_api_cuda', '2026.07.0', 'GPU-idem', TRUE, 'backend_proof_and_benchmark', 24576, TRUE, 'backend_telemetry_bound_to_verified_gpu', 'unobserved', 90, 10, 99, 'freshness_backend_timestamp_present', 'not_configured', '{}', 0, '[]', 'source_idem', $1, $1)",
                &[&now_text],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO workload_execution_profiles (workload_type, template_id, image_ref, status, created_at, updated_at) VALUES ('llm_realtime_api', 'llm_inference', $1, 'active', $2, $2)",
                &[&format!("ghcr.io/burd/runtime/llm@sha256:{}", "c".repeat(64)), &now_text],
            )
            .await
            .unwrap();
        let auth = crate::customer::CustomerApiKeyAuth {
            api_key_id: "api_key_idem".to_string(),
            organization_id: "org_idem".to_string(),
            project_id: Some("project_idem".to_string()),
            scopes: vec!["workloads:write".to_string()],
        };
        let original = request();
        let original_hash = burd_protocol::hash_canonical(&original).unwrap();
        let command = |request: CreateCustomerWorkloadRequest, request_hash: String| {
            CreateCustomerWorkloadCommand {
                request_id: "req_idem".to_string(),
                scope: "POST /v1/customer/projects/project_idem/workloads".to_string(),
                idempotency_key: "same-key".to_string(),
                request_hash,
                auth: auth.clone(),
                project_id: "project_idem".to_string(),
                request,
            }
        };
        let first = db
            .create_customer_workload_idempotently(
                command(original.clone(), original_hash.clone()),
                &crate::scheduler::tests::runtime_policy(),
            )
            .await
            .unwrap();
        let replay = db
            .create_customer_workload_idempotently(
                command(original, original_hash),
                &crate::scheduler::tests::runtime_policy(),
            )
            .await
            .unwrap();
        assert_eq!(first, replay);
        let mut conflict = request();
        conflict.requirements.minimum_vram_mib = Some(24_576);
        let conflict_hash = burd_protocol::hash_canonical(&conflict).unwrap();
        assert_eq!(
            db.create_customer_workload_idempotently(
                command(conflict, conflict_hash),
                &crate::scheduler::tests::runtime_policy(),
            )
            .await
            .unwrap(),
            CreateCustomerWorkloadOutcome::Conflict
        );
        let count: i64 = client
            .query_one("SELECT COUNT(*) AS count FROM customer_workloads", &[])
            .await
            .unwrap()
            .get("count");
        assert_eq!(count, 1);
    }
}
