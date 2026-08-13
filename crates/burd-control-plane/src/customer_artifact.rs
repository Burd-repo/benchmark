use crate::customer::CustomerApiKeyAuth;
use crate::db::{Database, DbError, IdempotencyRecord};
use crate::remote_session::SessionError;
use burd_protocol::{
    CUSTOMER_ARTIFACT_SCHEMA_VERSION, CUSTOMER_ARTIFACT_UPLOAD_INTENT_SCHEMA_VERSION,
    CreateCustomerArtifactRequest, CustomerArtifactRecord, CustomerArtifactResponse,
    CustomerArtifactUploadIntentResponse, CustomerArtifactUploadTarget,
};
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

pub const MAX_CUSTOMER_ARTIFACT_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const DEFAULT_ARTIFACT_RETENTION_SECONDS: u32 = 7 * 24 * 60 * 60;
const MAX_ARTIFACT_RETENTION_SECONDS: u32 = 30 * 24 * 60 * 60;
const UPLOAD_INTENT_TTL_SECONDS: i64 = 15 * 60;

#[derive(Debug, Clone)]
pub struct CreateCustomerArtifactCommand {
    pub request_id: String,
    pub scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub auth: CustomerApiKeyAuth,
    pub project_id: String,
    pub request: CreateCustomerArtifactRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateCustomerArtifactOutcome {
    Response(IdempotencyRecord),
    Conflict,
}

#[derive(Debug, Clone)]
pub struct AuthorizedCustomerArtifactUpload {
    pub artifact: CustomerArtifactRecord,
    pub object_key: String,
}

struct CustomerArtifactAudit<'a> {
    event_type: &'a str,
    summary: &'a str,
    metadata: serde_json::Value,
    occurred_at: &'a str,
}

impl Database {
    pub async fn create_customer_artifact_idempotently(
        &self,
        command: CreateCustomerArtifactCommand,
    ) -> Result<CreateCustomerArtifactOutcome, SessionError> {
        require_customer_scope(&command.auth, "artifacts:write")?;
        validate_id("project_id", &command.project_id, 128)?;
        validate_create_request(&command.request)?;

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
                Ok(CreateCustomerArtifactOutcome::Response(record))
            } else {
                Ok(CreateCustomerArtifactOutcome::Conflict)
            };
        }

        authorize_project_access(&transaction, &command.auth, &command.project_id).await?;
        let artifact_id = format!("artifact_{}", Uuid::new_v4());
        let object_key = format!("customer-artifacts/{artifact_id}/content");
        let upload_expires_at = (now + Duration::seconds(UPLOAD_INTENT_TTL_SECONDS)).to_rfc3339();
        let retention_seconds = command
            .request
            .retention_seconds
            .unwrap_or(DEFAULT_ARTIFACT_RETENTION_SECONDS);
        let expires_at = (now + Duration::seconds(i64::from(retention_seconds))).to_rfc3339();
        let size_bytes = to_i64(command.request.size_bytes)?;
        transaction
            .execute(
                "INSERT INTO customer_artifacts (artifact_id, organization_id, project_id, schema_version, client_artifact_id, status, object_key, sha256, size_bytes, content_type, upload_expires_at, expires_at, idempotency_key, request_hash, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 'pending_upload', $6, $7, $8, $9, $10, $11, $12, $13, $14, $14)",
                &[
                    &artifact_id,
                    &command.auth.organization_id,
                    &command.project_id,
                    &CUSTOMER_ARTIFACT_SCHEMA_VERSION,
                    &command.request.client_artifact_id,
                    &object_key,
                    &command.request.sha256.to_ascii_lowercase(),
                    &size_bytes,
                    &command.request.content_type,
                    &upload_expires_at,
                    &expires_at,
                    &command.idempotency_key,
                    &command.request_hash,
                    &now_text,
                ],
            )
            .await?;
        insert_artifact_audit_event(
            &transaction,
            &command.auth,
            &command.project_id,
            &artifact_id,
            CustomerArtifactAudit {
                event_type: "customer_artifact.upload_intent_created",
                summary: "customer artifact upload intent created",
                metadata: serde_json::json!({
                    "size_bytes": command.request.size_bytes,
                    "sha256": command.request.sha256.to_ascii_lowercase(),
                    "upload_expires_at": upload_expires_at,
                    "expires_at": expires_at,
                }),
                occurred_at: &now_text,
            },
        )
        .await?;
        let artifact = load_artifact(&transaction, &artifact_id).await?;
        let response_json = serde_json::to_string(&CustomerArtifactUploadIntentResponse {
            schema_version: CUSTOMER_ARTIFACT_UPLOAD_INTENT_SCHEMA_VERSION.to_string(),
            request_id: command.request_id,
            upload: upload_target(&artifact),
            artifact,
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
        Ok(CreateCustomerArtifactOutcome::Response(IdempotencyRecord {
            request_hash: command.request_hash,
            status_code: status_code as u16,
            response_json,
        }))
    }

    pub async fn authorize_customer_artifact_upload(
        &self,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
        artifact_id: &str,
    ) -> Result<AuthorizedCustomerArtifactUpload, SessionError> {
        require_customer_scope(auth, "artifacts:write")?;
        validate_id("project_id", project_id, 128)?;
        validate_id("artifact_id", artifact_id, 128)?;
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT artifact_id, organization_id, project_id, schema_version, client_artifact_id, status, object_key, sha256, size_bytes, content_type, upload_expires_at, expires_at, uploaded_at, ready_at, created_at, updated_at FROM customer_artifacts WHERE artifact_id = $1 AND project_id = $2 AND organization_id = $3",
                &[&artifact_id, &project_id, &auth.organization_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("customer artifact not found".to_string()))?;
        if auth
            .project_id
            .as_deref()
            .is_some_and(|bound| bound != project_id)
        {
            return Err(SessionError::Unauthorized);
        }
        let status: String = row.get("status");
        let upload_expires_at: String = row.get("upload_expires_at");
        if !matches!(status.as_str(), "pending_upload" | "uploaded") {
            return Err(SessionError::Conflict(
                "customer artifact state does not allow upload".to_string(),
            ));
        }
        if parse_timestamp(&upload_expires_at)? <= Utc::now() {
            return Err(SessionError::Expired);
        }
        let object_key = row.get("object_key");
        Ok(AuthorizedCustomerArtifactUpload {
            artifact: artifact_from_row(&row)?,
            object_key,
        })
    }

    pub async fn authorize_customer_artifact_finalize(
        &self,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
        artifact_id: &str,
    ) -> Result<AuthorizedCustomerArtifactUpload, SessionError> {
        require_customer_scope(auth, "artifacts:write")?;
        validate_id("project_id", project_id, 128)?;
        validate_id("artifact_id", artifact_id, 128)?;
        if auth
            .project_id
            .as_deref()
            .is_some_and(|bound| bound != project_id)
        {
            return Err(SessionError::Unauthorized);
        }
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT artifact_id, organization_id, project_id, schema_version, client_artifact_id, status, object_key, sha256, size_bytes, content_type, upload_expires_at, expires_at, uploaded_at, ready_at, created_at, updated_at FROM customer_artifacts WHERE artifact_id = $1 AND project_id = $2 AND organization_id = $3",
                &[&artifact_id, &project_id, &auth.organization_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("customer artifact not found".to_string()))?;
        let artifact = artifact_from_row(&row)?;
        if !matches!(artifact.status.as_str(), "uploaded" | "ready") {
            return Err(SessionError::Conflict(
                "customer artifact must be uploaded before finalize".to_string(),
            ));
        }
        if parse_timestamp(&artifact.expires_at)? <= Utc::now() {
            return Err(SessionError::Expired);
        }
        Ok(AuthorizedCustomerArtifactUpload {
            object_key: row.get("object_key"),
            artifact,
        })
    }

    pub async fn record_customer_artifact_upload(
        &self,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
        artifact_id: &str,
        sha256: &str,
        size_bytes: u64,
    ) -> Result<CustomerArtifactRecord, SessionError> {
        require_customer_scope(auth, "artifacts:write")?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = lock_owned_artifact(&transaction, auth, project_id, artifact_id).await?;
        let expected = artifact_from_row(&row)?;
        if !matches!(expected.status.as_str(), "pending_upload" | "uploaded") {
            return Err(SessionError::Conflict(
                "customer artifact state does not allow upload".to_string(),
            ));
        }
        if parse_timestamp(&expected.upload_expires_at)? <= Utc::now() {
            return Err(SessionError::Expired);
        }
        if sha256 != expected.sha256 || size_bytes != expected.size_bytes {
            return Err(SessionError::Invalid(
                "customer artifact upload does not match its declaration".to_string(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        let size_bytes = to_i64(size_bytes)?;
        transaction
            .execute(
                "UPDATE customer_artifacts SET status = 'uploaded', verified_sha256 = $1, verified_size_bytes = $2, uploaded_at = COALESCE(uploaded_at, $3), updated_at = $3 WHERE artifact_id = $4",
                &[&sha256, &size_bytes, &now, &artifact_id],
            )
            .await?;
        insert_artifact_audit_event(
            &transaction,
            auth,
            project_id,
            artifact_id,
            CustomerArtifactAudit {
                event_type: "customer_artifact.uploaded",
                summary: "customer artifact bytes uploaded and verified",
                metadata: serde_json::json!({"size_bytes": size_bytes, "sha256": sha256}),
                occurred_at: &now,
            },
        )
        .await?;
        let artifact = load_artifact(&transaction, artifact_id).await?;
        transaction.commit().await?;
        Ok(artifact)
    }

    pub async fn finalize_customer_artifact(
        &self,
        request_id: &str,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
        artifact_id: &str,
    ) -> Result<CustomerArtifactResponse, SessionError> {
        require_customer_scope(auth, "artifacts:write")?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = lock_owned_artifact(&transaction, auth, project_id, artifact_id).await?;
        let artifact = artifact_from_row(&row)?;
        if artifact.status == "ready" {
            transaction.commit().await?;
            return Ok(CustomerArtifactResponse {
                request_id: request_id.to_string(),
                artifact,
                duplicate: true,
            });
        }
        if artifact.status != "uploaded" {
            return Err(SessionError::Conflict(
                "customer artifact must be uploaded before finalize".to_string(),
            ));
        }
        if parse_timestamp(&artifact.expires_at)? <= Utc::now() {
            return Err(SessionError::Expired);
        }
        let verified_sha256: Option<String> = row.get("verified_sha256");
        let verified_size_bytes: Option<i64> = row.get("verified_size_bytes");
        if verified_sha256.as_deref() != Some(artifact.sha256.as_str())
            || verified_size_bytes.and_then(|value| u64::try_from(value).ok())
                != Some(artifact.size_bytes)
        {
            return Err(SessionError::Conflict(
                "customer artifact verified upload does not match declaration".to_string(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        transaction
            .execute(
                "UPDATE customer_artifacts SET status = 'ready', ready_at = $1, updated_at = $1 WHERE artifact_id = $2 AND status = 'uploaded'",
                &[&now, &artifact_id],
            )
            .await?;
        insert_artifact_audit_event(
            &transaction,
            auth,
            project_id,
            artifact_id,
            CustomerArtifactAudit {
                event_type: "customer_artifact.ready",
                summary: "customer artifact finalized and ready for workload binding",
                metadata: serde_json::json!({"sha256": artifact.sha256, "size_bytes": artifact.size_bytes}),
                occurred_at: &now,
            },
        )
        .await?;
        let artifact = load_artifact(&transaction, artifact_id).await?;
        transaction.commit().await?;
        Ok(CustomerArtifactResponse {
            request_id: request_id.to_string(),
            artifact,
            duplicate: false,
        })
    }
}

fn validate_create_request(request: &CreateCustomerArtifactRequest) -> Result<(), SessionError> {
    if let Some(value) = request.client_artifact_id.as_deref() {
        validate_id("client_artifact_id", value, 128)?;
    }
    validate_digest(&request.sha256)?;
    if request.size_bytes > MAX_CUSTOMER_ARTIFACT_BYTES {
        return Err(SessionError::Invalid(
            "customer artifact exceeds the maximum size".to_string(),
        ));
    }
    if let Some(content_type) = request.content_type.as_deref()
        && (content_type.is_empty()
            || content_type.len() > 255
            || content_type.chars().any(char::is_control))
    {
        return Err(SessionError::Invalid(
            "customer artifact content_type is invalid".to_string(),
        ));
    }
    if let Some(seconds) = request.retention_seconds
        && (seconds == 0 || seconds > MAX_ARTIFACT_RETENTION_SECONDS)
    {
        return Err(SessionError::Invalid(
            "customer artifact retention_seconds is outside allowed range".to_string(),
        ));
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), SessionError> {
    let valid = value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if valid {
        Ok(())
    } else {
        Err(SessionError::Invalid(
            "customer artifact sha256 must use sha256:<64 hex>".to_string(),
        ))
    }
}

async fn authorize_project_access(
    transaction: &Transaction<'_>,
    auth: &CustomerApiKeyAuth,
    project_id: &str,
) -> Result<(), SessionError> {
    if auth
        .project_id
        .as_deref()
        .is_some_and(|bound| bound != project_id)
    {
        return Err(SessionError::Unauthorized);
    }
    let found = transaction
        .query_opt(
            "SELECT project_id FROM projects WHERE project_id = $1 AND organization_id = $2 AND status = 'active'",
            &[&project_id, &auth.organization_id],
        )
        .await?
        .is_some();
    if found {
        Ok(())
    } else {
        Err(SessionError::Unauthorized)
    }
}

async fn lock_owned_artifact(
    transaction: &Transaction<'_>,
    auth: &CustomerApiKeyAuth,
    project_id: &str,
    artifact_id: &str,
) -> Result<Row, SessionError> {
    validate_id("project_id", project_id, 128)?;
    validate_id("artifact_id", artifact_id, 128)?;
    if auth
        .project_id
        .as_deref()
        .is_some_and(|bound| bound != project_id)
    {
        return Err(SessionError::Unauthorized);
    }
    transaction
        .query_opt(
            "SELECT artifact_id, organization_id, project_id, schema_version, client_artifact_id, status, object_key, sha256, size_bytes, content_type, upload_expires_at, expires_at, verified_sha256, verified_size_bytes, uploaded_at, ready_at, created_at, updated_at FROM customer_artifacts WHERE artifact_id = $1 AND project_id = $2 AND organization_id = $3 FOR UPDATE",
            &[&artifact_id, &project_id, &auth.organization_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("customer artifact not found".to_string()))
}

async fn load_artifact(
    transaction: &Transaction<'_>,
    artifact_id: &str,
) -> Result<CustomerArtifactRecord, SessionError> {
    let row = transaction
        .query_one(
            "SELECT artifact_id, organization_id, project_id, schema_version, client_artifact_id, status, sha256, size_bytes, content_type, upload_expires_at, expires_at, uploaded_at, ready_at, created_at, updated_at FROM customer_artifacts WHERE artifact_id = $1",
            &[&artifact_id],
        )
        .await?;
    artifact_from_row(&row)
}

fn artifact_from_row(row: &Row) -> Result<CustomerArtifactRecord, SessionError> {
    let size_bytes = row.get::<_, i64>("size_bytes");
    Ok(CustomerArtifactRecord {
        artifact_id: row.get("artifact_id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        schema_version: row.get("schema_version"),
        client_artifact_id: row.get("client_artifact_id"),
        status: row.get("status"),
        sha256: row.get("sha256"),
        size_bytes: u64::try_from(size_bytes)
            .map_err(|_| SessionError::Invalid("customer artifact size is invalid".to_string()))?,
        content_type: row.get("content_type"),
        upload_expires_at: row.get("upload_expires_at"),
        expires_at: row.get("expires_at"),
        uploaded_at: row.get("uploaded_at"),
        ready_at: row.get("ready_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn upload_target(artifact: &CustomerArtifactRecord) -> CustomerArtifactUploadTarget {
    CustomerArtifactUploadTarget {
        method: "PUT".to_string(),
        url: format!(
            "/v1/customer/projects/{}/artifacts/{}/content",
            artifact.project_id, artifact.artifact_id
        ),
        expires_at: artifact.upload_expires_at.clone(),
        content_length: artifact.size_bytes,
        sha256: artifact.sha256.clone(),
    }
}

async fn insert_artifact_audit_event(
    transaction: &Transaction<'_>,
    auth: &CustomerApiKeyAuth,
    project_id: &str,
    artifact_id: &str,
    audit: CustomerArtifactAudit<'_>,
) -> Result<(), SessionError> {
    transaction
        .execute(
            "INSERT INTO customer_audit_events (customer_audit_event_id, organization_id, project_id, schema_version, actor_type, actor_id, event_type, entity_type, entity_id, summary, metadata_json, occurred_at) VALUES ($1, $2, $3, 'burd-customer-audit-v1', 'customer_api_key', $4, $5, 'customer_artifact', $6, $7, $8, $9)",
            &[
                &format!("customer_audit_{}", Uuid::new_v4()),
                &auth.organization_id,
                &project_id,
                &auth.api_key_id,
                &audit.event_type,
                &artifact_id,
                &audit.summary,
                &audit.metadata.to_string(),
                &audit.occurred_at,
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

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, SessionError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| SessionError::Invalid("customer artifact timestamp is invalid".to_string()))
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

    fn request() -> CreateCustomerArtifactRequest {
        CreateCustomerArtifactRequest {
            client_artifact_id: Some("input_1".to_string()),
            sha256: format!("sha256:{}", "a".repeat(64)),
            size_bytes: 1024,
            content_type: Some("application/json".to_string()),
            retention_seconds: Some(3600),
        }
    }

    #[test]
    fn validates_digest_size_content_type_and_retention() {
        validate_create_request(&request()).unwrap();
        let mut invalid = request();
        invalid.sha256 = "sha256:short".to_string();
        assert!(validate_create_request(&invalid).is_err());
        invalid = request();
        invalid.size_bytes = MAX_CUSTOMER_ARTIFACT_BYTES + 1;
        assert!(validate_create_request(&invalid).is_err());
        invalid = request();
        invalid.content_type = Some("text/plain\nsecret".to_string());
        assert!(validate_create_request(&invalid).is_err());
        invalid = request();
        invalid.retention_seconds = Some(0);
        assert!(validate_create_request(&invalid).is_err());
    }

    async fn seed_project(client: &tokio_postgres::Client, suffix: &str, now: &str) {
        client
            .execute(
                "INSERT INTO organizations (organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ($1, 'burd-customer-organization-v1', 'Org', 'active', $2, $2)",
                &[&format!("org_{suffix}"), &now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO projects (project_id, organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ($1, $2, 'burd-customer-project-v1', 'Project', 'active', $3, $3)",
                &[
                    &format!("project_{suffix}"),
                    &format!("org_{suffix}"),
                    &now,
                ],
            )
            .await
            .unwrap();
    }

    fn auth(suffix: &str) -> CustomerApiKeyAuth {
        CustomerApiKeyAuth {
            api_key_id: format!("api_key_{suffix}"),
            organization_id: format!("org_{suffix}"),
            project_id: Some(format!("project_{suffix}")),
            scopes: vec!["artifacts:write".to_string()],
        }
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_artifact_lifecycle_is_idempotent_and_audited() {
        let db = crate::scheduler::tests::postgres_test_database("burd_customer_artifact").await;
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        seed_project(&client, "artifact", &now).await;
        let request = request();
        let request_hash = burd_protocol::hash_canonical(&request).unwrap();
        let command = || CreateCustomerArtifactCommand {
            request_id: "req_artifact".to_string(),
            scope: "POST /v1/customer/projects/project_artifact/artifacts".to_string(),
            idempotency_key: "artifact-key".to_string(),
            request_hash: request_hash.clone(),
            auth: auth("artifact"),
            project_id: "project_artifact".to_string(),
            request: request.clone(),
        };
        let first = db
            .create_customer_artifact_idempotently(command())
            .await
            .unwrap();
        let replay = db
            .create_customer_artifact_idempotently(command())
            .await
            .unwrap();
        assert_eq!(first, replay);
        let mut conflicting_request = request.clone();
        conflicting_request.size_bytes += 1;
        let conflict = db
            .create_customer_artifact_idempotently(CreateCustomerArtifactCommand {
                request_id: "req_artifact_conflict".to_string(),
                scope: "POST /v1/customer/projects/project_artifact/artifacts".to_string(),
                idempotency_key: "artifact-key".to_string(),
                request_hash: burd_protocol::hash_canonical(&conflicting_request).unwrap(),
                auth: auth("artifact"),
                project_id: "project_artifact".to_string(),
                request: conflicting_request,
            })
            .await
            .unwrap();
        assert_eq!(conflict, CreateCustomerArtifactOutcome::Conflict);
        let CreateCustomerArtifactOutcome::Response(record) = first else {
            panic!("expected artifact response");
        };
        let created: CustomerArtifactUploadIntentResponse =
            serde_json::from_str(&record.response_json).unwrap();
        assert_eq!(created.artifact.status, "pending_upload");
        assert!(!record.response_json.contains("object_key"));
        let uploaded = db
            .record_customer_artifact_upload(
                &auth("artifact"),
                "project_artifact",
                &created.artifact.artifact_id,
                &request.sha256,
                request.size_bytes,
            )
            .await
            .unwrap();
        assert_eq!(uploaded.status, "uploaded");
        let finalized = db
            .finalize_customer_artifact(
                "req_finalize",
                &auth("artifact"),
                "project_artifact",
                &created.artifact.artifact_id,
            )
            .await
            .unwrap();
        assert_eq!(finalized.artifact.status, "ready");
        assert!(!finalized.duplicate);
        let duplicate = db
            .finalize_customer_artifact(
                "req_finalize_again",
                &auth("artifact"),
                "project_artifact",
                &created.artifact.artifact_id,
            )
            .await
            .unwrap();
        assert!(duplicate.duplicate);
        let audit_count = client
            .query_one(
                "SELECT COUNT(*) AS count FROM customer_audit_events WHERE entity_id = $1",
                &[&created.artifact.artifact_id],
            )
            .await
            .unwrap()
            .get::<_, i64>("count");
        assert_eq!(audit_count, 3);
        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_artifact_rejects_cross_project_and_mismatched_upload() {
        let db =
            crate::scheduler::tests::postgres_test_database("burd_customer_artifact_auth").await;
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        seed_project(&client, "owner", &now).await;
        seed_project(&client, "other", &now).await;
        let request = request();
        let request_hash = burd_protocol::hash_canonical(&request).unwrap();
        let created = db
            .create_customer_artifact_idempotently(CreateCustomerArtifactCommand {
                request_id: "req_owner".to_string(),
                scope: "POST /v1/customer/projects/project_owner/artifacts".to_string(),
                idempotency_key: "owner-key".to_string(),
                request_hash,
                auth: auth("owner"),
                project_id: "project_owner".to_string(),
                request: request.clone(),
            })
            .await
            .unwrap();
        let CreateCustomerArtifactOutcome::Response(record) = created else {
            panic!("expected artifact response");
        };
        let created: CustomerArtifactUploadIntentResponse =
            serde_json::from_str(&record.response_json).unwrap();
        assert!(
            db.authorize_customer_artifact_upload(
                &auth("other"),
                "project_other",
                &created.artifact.artifact_id,
            )
            .await
            .is_err()
        );
        assert!(
            db.record_customer_artifact_upload(
                &auth("owner"),
                "project_owner",
                &created.artifact.artifact_id,
                &format!("sha256:{}", "b".repeat(64)),
                request.size_bytes,
            )
            .await
            .is_err()
        );
        db.drop_schema_for_test().await.unwrap();
    }
}
