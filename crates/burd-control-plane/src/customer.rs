use crate::db::{Database, DbError, IdempotencyRecord, NewAuditEvent, insert_audit_event};
use crate::remote_session::SessionError;
use burd_protocol::{
    CUSTOMER_API_KEY_SCHEMA_VERSION, CUSTOMER_AUDIT_SCHEMA_VERSION,
    CUSTOMER_CREDIT_LEDGER_SCHEMA_VERSION, CUSTOMER_ORGANIZATION_SCHEMA_VERSION,
    CUSTOMER_PROJECT_SCHEMA_VERSION, CancelReservationRequest, CreateCustomerApiKeyRequest,
    CreateCustomerApiKeyResponse, CreateCustomerUserRequest, CreateOrganizationRequest,
    CreateProjectRequest, CreateReservationRequest, CustomerApiKeyRecord, CustomerAuditEventRecord,
    CustomerCreditLedgerEntry, CustomerCreditLedgerResponse, CustomerUsageResponse,
    CustomerUsageSummary, CustomerUserRecord, CustomerUserResponse, GrantCustomerCreditsRequest,
    ListCustomerAuditEventsResponse, ListMarketplaceReservationsResponse,
    MARKETPLACE_RESERVATION_SCHEMA_VERSION, MarketplaceReservationRecord,
    MarketplaceReservationResponse, OrganizationMembershipRecord, OrganizationRecord,
    OrganizationResponse, ProjectQuotaRecord, ProjectQuotaResponse, ProjectRecord, ProjectResponse,
    UpsertProjectQuotaRequest, random_token, sha256_hex,
};
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const DEFAULT_MAX_ACTIVE_RESERVATIONS: u32 = 2;
const DEFAULT_MAX_RESERVED_GPU_SECONDS: u64 = 8 * 60 * 60;
const DEFAULT_MAX_RESERVATION_TTL_SECONDS: u32 = 60 * 60;
const MAX_RESERVATION_TTL_SECONDS: u32 = 24 * 60 * 60;
const MAX_RESERVATION_LIST_LIMIT: u32 = 200;
const MAX_CUSTOMER_AUDIT_LIMIT: u32 = 200;
const DEFAULT_CUSTOMER_SCOPES: &[&str] = &[
    "billing:read",
    "billing:write",
    "reservations:read",
    "reservations:write",
    "usage:read",
];
const ALLOWED_CUSTOMER_SCOPES: &[&str] = &[
    "billing:read",
    "billing:write",
    "reservations:read",
    "reservations:write",
    "usage:read",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerApiKeyAuth {
    pub api_key_id: String,
    pub organization_id: String,
    pub project_id: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateReservationCommand {
    pub request_id: String,
    pub scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub auth: CustomerApiKeyAuth,
    pub project_id: String,
    pub request: CreateReservationRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateReservationOutcome {
    Response(IdempotencyRecord),
    Conflict,
}

#[derive(Debug, Clone)]
struct ProjectAccess {
    organization_id: String,
    project_id: String,
    max_active_reservations: u32,
    max_reserved_gpu_seconds: u64,
    max_reservation_ttl_seconds: u32,
}

#[derive(Debug, Clone)]
struct ListingReservationSource {
    listing_id: String,
    provider_id: String,
    device_id: String,
    session_id: Option<String>,
    status: String,
    current_status: String,
    workload_type: String,
    gpu_uuid: Option<String>,
}

#[derive(Debug, Clone)]
struct NewCustomerAuditEvent<'a> {
    organization_id: &'a str,
    project_id: Option<String>,
    actor_type: &'a str,
    actor_id: Option<String>,
    event_type: &'a str,
    entity_type: &'a str,
    entity_id: &'a str,
    summary: &'a str,
    metadata_json: &'a str,
}

impl Database {
    pub async fn create_customer_user(
        &self,
        request_id: &str,
        request: &CreateCustomerUserRequest,
    ) -> Result<CustomerUserResponse, SessionError> {
        validate_customer_user_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        let user = CustomerUserRecord {
            user_id: format!("user_{}", Uuid::new_v4()),
            email: request.email.clone(),
            status: request
                .status
                .clone()
                .unwrap_or_else(|| "active".to_string()),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        transaction
            .execute(
                "INSERT INTO users (user_id, email, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5)",
                &[&user.user_id, &user.email, &user.status, &user.created_at, &user.updated_at],
            )
            .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "user",
                entity_id: &user.user_id,
                event_type: "customer_user.created",
                idempotency_key: None,
                summary: "customer human identity created",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(CustomerUserResponse {
            request_id: request_id.to_string(),
            user,
        })
    }

    pub async fn create_organization(
        &self,
        request_id: &str,
        request: &CreateOrganizationRequest,
    ) -> Result<OrganizationResponse, SessionError> {
        validate_organization_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        if let Some(owner_user_id) = request.owner_user_id.as_deref() {
            require_user_exists(&transaction, owner_user_id).await?;
        }
        let organization = OrganizationRecord {
            organization_id: format!("org_{}", Uuid::new_v4()),
            schema_version: CUSTOMER_ORGANIZATION_SCHEMA_VERSION.to_string(),
            display_name: request.display_name.clone(),
            status: "active".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        transaction
            .execute(
                "INSERT INTO organizations (organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    &organization.organization_id,
                    &organization.schema_version,
                    &organization.display_name,
                    &organization.status,
                    &organization.created_at,
                    &organization.updated_at,
                ],
            )
            .await?;
        let membership = if let Some(owner_user_id) = request.owner_user_id.as_deref() {
            transaction
                .execute(
                    "INSERT INTO organization_users (organization_id, user_id, role, status, created_at, updated_at) VALUES ($1, $2, 'owner', 'active', $3, $3)",
                    &[&organization.organization_id, &owner_user_id, &now],
                )
                .await?;
            Some(OrganizationMembershipRecord {
                organization_id: organization.organization_id.clone(),
                user_id: owner_user_id.to_string(),
                role: "owner".to_string(),
                status: "active".to_string(),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
        } else {
            None
        };
        insert_customer_audit_event(
            &transaction,
            NewCustomerAuditEvent {
                organization_id: &organization.organization_id,
                project_id: None,
                actor_type: "admin",
                actor_id: None,
                event_type: "organization.created",
                entity_type: "organization",
                entity_id: &organization.organization_id,
                summary: "customer organization created",
                metadata_json: "{}",
            },
        )
        .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "organization",
                entity_id: &organization.organization_id,
                event_type: "organization.created",
                idempotency_key: None,
                summary: "customer organization created",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(OrganizationResponse {
            request_id: request_id.to_string(),
            organization,
            membership,
        })
    }

    pub async fn get_organization(
        &self,
        request_id: &str,
        organization_id: &str,
    ) -> Result<OrganizationResponse, SessionError> {
        validate_id("organization_id", organization_id, 128)?;
        let client = self.connect().await?;
        let row = client
            .query_opt(
                &format!(
                    "{} WHERE organization_id = $1",
                    organization_select_columns()
                ),
                &[&organization_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("organization not found".to_string()))?;
        Ok(OrganizationResponse {
            request_id: request_id.to_string(),
            organization: organization_from_row(row),
            membership: None,
        })
    }
    pub async fn create_project(
        &self,
        request_id: &str,
        organization_id: &str,
        request: &CreateProjectRequest,
    ) -> Result<ProjectResponse, SessionError> {
        validate_id("organization_id", organization_id, 128)?;
        validate_project_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        require_active_organization(&transaction, organization_id).await?;
        let now = Utc::now().to_rfc3339();
        let project = ProjectRecord {
            project_id: format!("project_{}", Uuid::new_v4()),
            organization_id: organization_id.to_string(),
            schema_version: CUSTOMER_PROJECT_SCHEMA_VERSION.to_string(),
            display_name: request.display_name.clone(),
            status: "active".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        transaction
            .execute(
                "INSERT INTO projects (project_id, organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6)",
                &[
                    &project.project_id,
                    &project.organization_id,
                    &project.schema_version,
                    &project.display_name,
                    &project.status,
                    &project.created_at,
                ],
            )
            .await?;
        transaction
            .execute(
                "INSERT INTO project_quotas (project_id, max_active_reservations, max_reserved_gpu_seconds, max_reservation_ttl_seconds, updated_at) VALUES ($1, $2, $3, $4, $5)",
                &[
                    &project.project_id,
                    &(DEFAULT_MAX_ACTIVE_RESERVATIONS as i32),
                    &to_i64(DEFAULT_MAX_RESERVED_GPU_SECONDS)?,
                    &(DEFAULT_MAX_RESERVATION_TTL_SECONDS as i32),
                    &now,
                ],
            )
            .await?;
        insert_customer_audit_event(
            &transaction,
            NewCustomerAuditEvent {
                organization_id,
                project_id: Some(project.project_id.clone()),
                actor_type: "admin",
                actor_id: None,
                event_type: "project.created",
                entity_type: "project",
                entity_id: &project.project_id,
                summary: "customer project created",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(ProjectResponse {
            request_id: request_id.to_string(),
            project,
        })
    }

    pub async fn upsert_project_quota(
        &self,
        request_id: &str,
        project_id: &str,
        request: &UpsertProjectQuotaRequest,
    ) -> Result<ProjectQuotaResponse, SessionError> {
        validate_id("project_id", project_id, 128)?;
        validate_quota_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let organization_id = require_active_project(&transaction, project_id).await?;
        let now = Utc::now().to_rfc3339();
        transaction
            .execute(
                "INSERT INTO project_quotas (project_id, max_active_reservations, max_reserved_gpu_seconds, max_reservation_ttl_seconds, updated_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (project_id) DO UPDATE SET max_active_reservations = EXCLUDED.max_active_reservations, max_reserved_gpu_seconds = EXCLUDED.max_reserved_gpu_seconds, max_reservation_ttl_seconds = EXCLUDED.max_reservation_ttl_seconds, updated_at = EXCLUDED.updated_at",
                &[
                    &project_id,
                    &(request.max_active_reservations as i32),
                    &to_i64(request.max_reserved_gpu_seconds)?,
                    &(request.max_reservation_ttl_seconds as i32),
                    &now,
                ],
            )
            .await?;
        let quota = load_project_quota(&transaction, project_id).await?;
        insert_customer_audit_event(
            &transaction,
            NewCustomerAuditEvent {
                organization_id: &organization_id,
                project_id: Some(project_id.to_string()),
                actor_type: "admin",
                actor_id: None,
                event_type: "project_quota.updated",
                entity_type: "project_quota",
                entity_id: project_id,
                summary: "customer project quota updated",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(ProjectQuotaResponse {
            request_id: request_id.to_string(),
            quota,
        })
    }

    pub async fn create_customer_api_key(
        &self,
        request_id: &str,
        project_id: &str,
        request: &CreateCustomerApiKeyRequest,
    ) -> Result<CreateCustomerApiKeyResponse, SessionError> {
        validate_id("project_id", project_id, 128)?;
        let scopes = normalized_scopes(&request.scopes)?;
        if let Some(expires_at) = request.expires_at.as_deref() {
            validate_future_timestamp("expires_at", expires_at)?;
        }
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let organization_id = require_active_project(&transaction, project_id).await?;
        let now = Utc::now().to_rfc3339();
        let token = random_token("burd_customer").map_err(SessionError::Invalid)?;
        let key_hash = sha256_hex(token.as_bytes());
        let key_prefix = token.chars().take(24).collect::<String>();
        let scopes_json = serde_json::to_string(&scopes)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
        let api_key_id = format!("cak_{}", Uuid::new_v4());
        transaction
            .execute(
                "INSERT INTO customer_api_keys (api_key_id, organization_id, project_id, schema_version, key_prefix, key_hash, status, scopes_json, created_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, 'active', $7, $8, $9)",
                &[
                    &api_key_id,
                    &organization_id,
                    &Some(project_id.to_string()),
                    &CUSTOMER_API_KEY_SCHEMA_VERSION,
                    &key_prefix,
                    &key_hash,
                    &scopes_json,
                    &now,
                    &request.expires_at,
                ],
            )
            .await?;
        let api_key = load_customer_api_key(&transaction, &api_key_id).await?;
        insert_customer_audit_event(
            &transaction,
            NewCustomerAuditEvent {
                organization_id: &organization_id,
                project_id: Some(project_id.to_string()),
                actor_type: "admin",
                actor_id: None,
                event_type: "customer_api_key.created",
                entity_type: "customer_api_key",
                entity_id: &api_key_id,
                summary: "customer API key created; plaintext token returned once",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(CreateCustomerApiKeyResponse {
            request_id: request_id.to_string(),
            api_key,
            token,
        })
    }

    pub async fn authorize_customer_api_key(
        &self,
        token: &str,
        project_id: Option<&str>,
    ) -> Result<CustomerApiKeyAuth, SessionError> {
        let key_hash = sha256_hex(token.as_bytes());
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_opt(
                "SELECT k.api_key_id, k.organization_id, k.project_id, k.status, k.scopes_json, k.expires_at, o.status AS organization_status, p.status AS project_status FROM customer_api_keys k JOIN organizations o ON o.organization_id = k.organization_id LEFT JOIN projects p ON p.project_id = k.project_id WHERE k.key_hash = $1 FOR UPDATE OF k",
                &[&key_hash],
            )
            .await?
            .ok_or(SessionError::Unauthorized)?;
        let api_key_id: String = row.get("api_key_id");
        let organization_id: String = row.get("organization_id");
        let bound_project_id: Option<String> = row.get("project_id");
        let key_status: String = row.get("status");
        let organization_status: String = row.get("organization_status");
        let project_status: Option<String> = row.get("project_status");
        if key_status != "active" || organization_status != "active" {
            return Err(SessionError::Unauthorized);
        }
        if let Some(status) = project_status.as_deref()
            && status != "active"
        {
            return Err(SessionError::Unauthorized);
        }
        if let Some(expires_at) = row.get::<_, Option<String>>("expires_at") {
            let expires_at = parse_timestamp("expires_at", &expires_at)?;
            if expires_at <= Utc::now() {
                return Err(SessionError::Expired);
            }
        }
        if let Some(project_id) = project_id {
            validate_id("project_id", project_id, 128)?;
            if bound_project_id
                .as_deref()
                .is_some_and(|bound| bound != project_id)
            {
                return Err(SessionError::Unauthorized);
            }
            let project_org = require_active_project(&transaction, project_id).await?;
            if project_org != organization_id {
                return Err(SessionError::Unauthorized);
            }
        }
        let scopes_json: String = row.get("scopes_json");
        let scopes: Vec<String> = serde_json::from_str(&scopes_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
        transaction
            .execute(
                "UPDATE customer_api_keys SET last_used_at = $1 WHERE api_key_id = $2",
                &[&Utc::now().to_rfc3339(), &api_key_id],
            )
            .await?;
        transaction.commit().await?;
        Ok(CustomerApiKeyAuth {
            api_key_id,
            organization_id,
            project_id: bound_project_id,
            scopes,
        })
    }

    pub async fn grant_customer_credits(
        &self,
        request_id: &str,
        project_id: &str,
        request: &GrantCustomerCreditsRequest,
    ) -> Result<CustomerCreditLedgerResponse, SessionError> {
        validate_id("project_id", project_id, 128)?;
        validate_credit_request(request)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let organization_id = require_active_project(&transaction, project_id).await?;
        let entry_type = if request.amount_credits >= 0 {
            "credit_grant"
        } else {
            "credit_adjustment"
        };
        let entry = append_customer_credit_ledger_entry(
            &transaction,
            &organization_id,
            project_id,
            entry_type,
            request.amount_credits,
            None,
            &request.reason,
        )
        .await?;
        insert_customer_audit_event(
            &transaction,
            NewCustomerAuditEvent {
                organization_id: &organization_id,
                project_id: Some(project_id.to_string()),
                actor_type: "admin",
                actor_id: None,
                event_type: "customer_credits.granted",
                entity_type: "customer_credit_ledger_entry",
                entity_id: &entry.credit_entry_id,
                summary: "customer credit ledger entry appended",
                metadata_json: "{}",
            },
        )
        .await?;
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "customer_credit_ledger_entry",
                entity_id: &entry.credit_entry_id,
                event_type: "customer_credits.granted",
                idempotency_key: None,
                summary: "customer credit ledger entry appended",
                metadata_json: "{}",
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(CustomerCreditLedgerResponse {
            request_id: request_id.to_string(),
            entry,
        })
    }
    pub async fn create_marketplace_reservation_idempotently(
        &self,
        command: CreateReservationCommand,
    ) -> Result<CreateReservationOutcome, SessionError> {
        require_customer_scope(&command.auth, "reservations:write")?;
        validate_id("project_id", &command.project_id, 128)?;
        validate_reservation_request(&command.request)?;
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
                Ok(CreateReservationOutcome::Response(record))
            } else {
                Ok(CreateReservationOutcome::Conflict)
            };
        }

        expire_stale_reservations(&transaction, &now_text).await?;
        let project =
            authorize_project_access(&transaction, &command.auth, &command.project_id).await?;
        let listing =
            load_listing_for_reservation(&transaction, &command.request.listing_id).await?;
        if let Some(workload_type) = command.request.workload_type.as_deref()
            && workload_type != listing.workload_type
        {
            return Err(SessionError::Conflict(
                "reservation workload_type does not match listing".to_string(),
            ));
        }
        assert_listing_reservable(&listing)?;
        assert_project_quota(
            &transaction,
            &project,
            command.request.duration_seconds,
            &now_text,
        )
        .await?;

        let starts_at = reservation_start_time(command.request.starts_at.as_deref(), now)?;
        let expires_at = starts_at + Duration::seconds(i64::from(command.request.duration_seconds));
        let reservation_id = format!("reservation_{}", Uuid::new_v4());
        let reason_codes = vec![
            "marketplace_listing_backend_published".to_string(),
            "customer_project_quota_satisfied".to_string(),
            "customer_api_key_authorized".to_string(),
        ];
        let reason_codes_json = serde_json::to_string(&reason_codes)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
        transaction
            .execute(
                "INSERT INTO marketplace_reservations (reservation_id, organization_id, project_id, listing_id, provider_id, device_id, session_id, schema_version, workload_type, gpu_uuid, status, idempotency_key, request_hash, starts_at, expires_at, reserved_gpu_seconds, reason_codes_json, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'reserved', $11, $12, $13, $14, $15, $16, $17, $17)",
                &[
                    &reservation_id,
                    &project.organization_id,
                    &project.project_id,
                    &listing.listing_id,
                    &listing.provider_id,
                    &listing.device_id,
                    &listing.session_id,
                    &MARKETPLACE_RESERVATION_SCHEMA_VERSION,
                    &listing.workload_type,
                    &listing.gpu_uuid,
                    &command.idempotency_key,
                    &command.request_hash,
                    &starts_at.to_rfc3339(),
                    &expires_at.to_rfc3339(),
                    &to_i64(u64::from(command.request.duration_seconds))?,
                    &reason_codes_json,
                    &now_text,
                ],
            )
            .await?;
        let availability_window_json = serde_json::json!({
            "mode": "customer_reservation_hold",
            "source": "customer_reservation",
            "reservations_enabled": false,
            "active_reservation_id": reservation_id,
        })
        .to_string();
        transaction
            .execute(
                "UPDATE marketplace_listings SET current_status = 'reserved', availability_window_json = $1, updated_at = $2 WHERE listing_id = $3",
                &[&availability_window_json, &now_text, &listing.listing_id],
            )
            .await?;
        append_customer_credit_ledger_entry(
            &transaction,
            &project.organization_id,
            &project.project_id,
            "reservation_hold",
            0,
            Some(&reservation_id),
            "reservation hold recorded without credit debit in BN-17",
        )
        .await?;
        let reservation = load_reservation(&transaction, &reservation_id).await?;
        let metadata_json = serde_json::json!({
            "listing_id": reservation.listing_id,
            "provider_id": reservation.provider_id,
            "device_id": reservation.device_id,
            "workload_type": reservation.workload_type,
            "duration_seconds": command.request.duration_seconds,
        })
        .to_string();
        insert_customer_audit_event(
            &transaction,
            NewCustomerAuditEvent {
                organization_id: &project.organization_id,
                project_id: Some(project.project_id.clone()),
                actor_type: "customer_api_key",
                actor_id: Some(command.auth.api_key_id.clone()),
                event_type: "marketplace_reservation.created",
                entity_type: "marketplace_reservation",
                entity_id: &reservation.reservation_id,
                summary: "customer marketplace reservation created",
                metadata_json: &metadata_json,
            },
        )
        .await?;
        let response_json = serde_json::to_string(&MarketplaceReservationResponse {
            request_id: command.request_id,
            reservation,
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
        Ok(CreateReservationOutcome::Response(IdempotencyRecord {
            request_hash: command.request_hash,
            status_code: status_code as u16,
            response_json,
        }))
    }

    pub async fn cancel_marketplace_reservation(
        &self,
        request_id: &str,
        auth: &CustomerApiKeyAuth,
        reservation_id: &str,
        request: &CancelReservationRequest,
    ) -> Result<MarketplaceReservationResponse, SessionError> {
        require_customer_scope(auth, "reservations:write")?;
        validate_id("reservation_id", reservation_id, 160)?;
        if let Some(reason) = request.reason.as_deref()
            && !is_bounded_ascii(reason, 256)
        {
            return Err(SessionError::Invalid(
                "reservation cancellation reason must be printable ASCII".to_string(),
            ));
        }
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        expire_stale_reservations(&transaction, &now).await?;
        let before = load_reservation(&transaction, reservation_id).await?;
        authorize_project_access(&transaction, auth, &before.project_id).await?;
        if before.status == "reserved" {
            transaction
                .execute(
                    "UPDATE marketplace_reservations SET status = 'cancelled', cancelled_at = $1, updated_at = $1 WHERE reservation_id = $2 AND status = 'reserved'",
                    &[&now, &reservation_id],
                )
                .await?;
            append_customer_credit_ledger_entry(
                &transaction,
                &before.organization_id,
                &before.project_id,
                "reservation_release",
                0,
                Some(reservation_id),
                "reservation hold released without credit settlement in BN-17",
            )
            .await?;
            insert_customer_audit_event(
                &transaction,
                NewCustomerAuditEvent {
                    organization_id: &before.organization_id,
                    project_id: Some(before.project_id.clone()),
                    actor_type: "customer_api_key",
                    actor_id: Some(auth.api_key_id.clone()),
                    event_type: "marketplace_reservation.cancelled",
                    entity_type: "marketplace_reservation",
                    entity_id: reservation_id,
                    summary: "customer marketplace reservation cancelled",
                    metadata_json: &serde_json::json!({ "reason": request.reason }).to_string(),
                },
            )
            .await?;
        }
        let reservation = load_reservation(&transaction, reservation_id).await?;
        transaction.commit().await?;
        Ok(MarketplaceReservationResponse {
            request_id: request_id.to_string(),
            reservation,
            duplicate: before.status != "reserved",
        })
    }

    pub async fn list_project_reservations(
        &self,
        request_id: &str,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
        limit: u32,
    ) -> Result<ListMarketplaceReservationsResponse, SessionError> {
        require_customer_scope(auth, "reservations:read")?;
        validate_id("project_id", project_id, 128)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        expire_stale_reservations(&transaction, &now).await?;
        authorize_project_access(&transaction, auth, project_id).await?;
        let limit = limit.clamp(1, MAX_RESERVATION_LIST_LIMIT) as i64;
        let rows = transaction
            .query(
                &format!(
                    "{} WHERE project_id = $1 ORDER BY updated_at DESC LIMIT $2",
                    reservation_select_columns()
                ),
                &[&project_id, &limit],
            )
            .await?;
        let reservations = rows
            .into_iter()
            .map(reservation_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await?;
        Ok(ListMarketplaceReservationsResponse {
            request_id: request_id.to_string(),
            reservations,
        })
    }

    pub async fn customer_project_usage(
        &self,
        request_id: &str,
        auth: &CustomerApiKeyAuth,
        project_id: &str,
    ) -> Result<CustomerUsageResponse, SessionError> {
        require_customer_scope(auth, "usage:read")?;
        validate_id("project_id", project_id, 128)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        expire_stale_reservations(&transaction, &now).await?;
        authorize_project_access(&transaction, auth, project_id).await?;
        let row = transaction
            .query_one(
                "SELECT COUNT(*)::BIGINT AS total_reservations, COUNT(*) FILTER (WHERE status = 'reserved')::BIGINT AS active_reservations, COALESCE(SUM(CASE WHEN status = 'reserved' THEN reserved_gpu_seconds ELSE 0 END), 0)::BIGINT AS reserved_gpu_seconds, COUNT(*) FILTER (WHERE status = 'cancelled')::BIGINT AS cancelled_reservations, COUNT(*) FILTER (WHERE status = 'expired')::BIGINT AS expired_reservations FROM marketplace_reservations WHERE project_id = $1",
                &[&project_id],
            )
            .await?;
        let credit_balance: i64 = transaction
            .query_one(
                "SELECT COALESCE(SUM(amount_credits), 0)::BIGINT FROM customer_credit_ledger_entries WHERE project_id = $1",
                &[&project_id],
            )
            .await?
            .get(0);
        transaction.commit().await?;
        Ok(CustomerUsageResponse {
            request_id: request_id.to_string(),
            usage: CustomerUsageSummary {
                project_id: project_id.to_string(),
                active_reservations: from_i64_to_u32(row.get("active_reservations"))?,
                reserved_gpu_seconds: from_i64_to_u64(row.get("reserved_gpu_seconds"))?,
                total_reservations: from_i64_to_u32(row.get("total_reservations"))?,
                cancelled_reservations: from_i64_to_u32(row.get("cancelled_reservations"))?,
                expired_reservations: from_i64_to_u32(row.get("expired_reservations"))?,
                credit_balance,
            },
        })
    }

    pub async fn list_customer_audit_events(
        &self,
        request_id: &str,
        organization_id: &str,
        limit: u32,
    ) -> Result<ListCustomerAuditEventsResponse, SessionError> {
        validate_id("organization_id", organization_id, 128)?;
        let limit = limit.clamp(1, MAX_CUSTOMER_AUDIT_LIMIT) as i64;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE organization_id = $1 ORDER BY occurred_at DESC LIMIT $2",
                    customer_audit_select_columns()
                ),
                &[&organization_id, &limit],
            )
            .await?;
        let events = rows
            .into_iter()
            .map(customer_audit_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListCustomerAuditEventsResponse {
            request_id: request_id.to_string(),
            events,
        })
    }
}
async fn require_user_exists(
    transaction: &Transaction<'_>,
    user_id: &str,
) -> Result<(), SessionError> {
    validate_id("user_id", user_id, 128)?;
    let exists = transaction
        .query_opt("SELECT user_id FROM users WHERE user_id = $1", &[&user_id])
        .await?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(SessionError::NotFound("user not found".to_string()))
    }
}

async fn require_active_organization(
    transaction: &Transaction<'_>,
    organization_id: &str,
) -> Result<(), SessionError> {
    let row = transaction
        .query_opt(
            "SELECT status FROM organizations WHERE organization_id = $1",
            &[&organization_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("organization not found".to_string()))?;
    let status: String = row.get("status");
    if status == "active" {
        Ok(())
    } else {
        Err(SessionError::Conflict(
            "organization is not active".to_string(),
        ))
    }
}

async fn require_active_project(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> Result<String, SessionError> {
    let row = transaction
        .query_opt(
            "SELECT p.organization_id, p.status AS project_status, o.status AS organization_status FROM projects p JOIN organizations o ON o.organization_id = p.organization_id WHERE p.project_id = $1",
            &[&project_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("project not found".to_string()))?;
    let project_status: String = row.get("project_status");
    let organization_status: String = row.get("organization_status");
    if project_status == "active" && organization_status == "active" {
        Ok(row.get("organization_id"))
    } else {
        Err(SessionError::Conflict(
            "project or organization is not active".to_string(),
        ))
    }
}

async fn authorize_project_access(
    transaction: &Transaction<'_>,
    auth: &CustomerApiKeyAuth,
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
            "SELECT p.project_id, p.organization_id, p.status AS project_status, o.status AS organization_status, q.max_active_reservations, q.max_reserved_gpu_seconds, q.max_reservation_ttl_seconds FROM projects p JOIN organizations o ON o.organization_id = p.organization_id JOIN project_quotas q ON q.project_id = p.project_id WHERE p.project_id = $1 FOR UPDATE OF p, q",
            &[&project_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("project not found".to_string()))?;
    let organization_id: String = row.get("organization_id");
    let project_status: String = row.get("project_status");
    let organization_status: String = row.get("organization_status");
    if organization_id != auth.organization_id {
        return Err(SessionError::Unauthorized);
    }
    if project_status != "active" || organization_status != "active" {
        return Err(SessionError::Conflict(
            "project or organization is not active".to_string(),
        ));
    }
    Ok(ProjectAccess {
        organization_id,
        project_id: row.get("project_id"),
        max_active_reservations: from_i32_to_u32(row.get("max_active_reservations"))?,
        max_reserved_gpu_seconds: from_i64_to_u64(row.get("max_reserved_gpu_seconds"))?,
        max_reservation_ttl_seconds: from_i32_to_u32(row.get("max_reservation_ttl_seconds"))?,
    })
}

async fn assert_project_quota(
    transaction: &Transaction<'_>,
    project: &ProjectAccess,
    duration_seconds: u32,
    now: &str,
) -> Result<(), SessionError> {
    if duration_seconds > project.max_reservation_ttl_seconds {
        return Err(SessionError::Conflict(
            "reservation duration exceeds project TTL quota".to_string(),
        ));
    }
    let row = transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT AS active_count, COALESCE(SUM(reserved_gpu_seconds), 0)::BIGINT AS active_reserved_gpu_seconds FROM marketplace_reservations WHERE project_id = $1 AND status = 'reserved' AND expires_at > $2",
            &[&project.project_id, &now],
        )
        .await?;
    let active_count = from_i64_to_u32(row.get("active_count"))?;
    let active_reserved_gpu_seconds = from_i64_to_u64(row.get("active_reserved_gpu_seconds"))?;
    if active_count >= project.max_active_reservations {
        return Err(SessionError::Conflict(
            "project active reservation quota exceeded".to_string(),
        ));
    }
    let requested = u64::from(duration_seconds);
    if active_reserved_gpu_seconds.saturating_add(requested) > project.max_reserved_gpu_seconds {
        return Err(SessionError::Conflict(
            "project reserved GPU-second quota exceeded".to_string(),
        ));
    }
    Ok(())
}

async fn load_listing_for_reservation(
    transaction: &Transaction<'_>,
    listing_id: &str,
) -> Result<ListingReservationSource, SessionError> {
    validate_id("listing_id", listing_id, 160)?;
    let row = transaction
        .query_opt(
            "SELECT listing_id, provider_id, device_id, session_id, status, current_status, workload_type, gpu_uuid FROM marketplace_listings WHERE listing_id = $1 FOR UPDATE",
            &[&listing_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("marketplace listing not found".to_string()))?;
    Ok(ListingReservationSource {
        listing_id: row.get("listing_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        status: row.get("status"),
        current_status: row.get("current_status"),
        workload_type: row.get("workload_type"),
        gpu_uuid: row.get("gpu_uuid"),
    })
}

fn assert_listing_reservable(listing: &ListingReservationSource) -> Result<(), SessionError> {
    if !matches!(listing.status.as_str(), "published" | "limited") {
        return Err(SessionError::Conflict(
            "marketplace listing is not published for reservation".to_string(),
        ));
    }
    if !matches!(listing.current_status.as_str(), "available" | "degraded") {
        return Err(SessionError::Conflict(
            "marketplace listing is not currently reservable".to_string(),
        ));
    }
    Ok(())
}

async fn expire_stale_reservations(
    transaction: &Transaction<'_>,
    now: &str,
) -> Result<u64, SessionError> {
    Ok(transaction
        .execute(
            "UPDATE marketplace_reservations SET status = 'expired', updated_at = $1 WHERE status = 'reserved' AND expires_at <= $1",
            &[&now],
        )
        .await?)
}

async fn append_customer_credit_ledger_entry(
    transaction: &Transaction<'_>,
    organization_id: &str,
    project_id: &str,
    entry_type: &str,
    amount_credits: i64,
    reservation_id: Option<&str>,
    reason: &str,
) -> Result<CustomerCreditLedgerEntry, SessionError> {
    validate_id("entry_type", entry_type, 64)?;
    if !is_bounded_ascii(reason, 256) {
        return Err(SessionError::Invalid(
            "credit ledger reason must be printable ASCII".to_string(),
        ));
    }
    let entry_id = format!("credit_{}", Uuid::new_v4());
    let now = Utc::now().to_rfc3339();
    transaction
        .execute(
            "INSERT INTO customer_credit_ledger_entries (credit_entry_id, organization_id, project_id, schema_version, entry_type, amount_credits, reservation_id, reason, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &entry_id,
                &organization_id,
                &project_id,
                &CUSTOMER_CREDIT_LEDGER_SCHEMA_VERSION,
                &entry_type,
                &amount_credits,
                &reservation_id,
                &reason,
                &now,
            ],
        )
        .await?;
    Ok(CustomerCreditLedgerEntry {
        credit_entry_id: entry_id,
        organization_id: organization_id.to_string(),
        project_id: project_id.to_string(),
        schema_version: CUSTOMER_CREDIT_LEDGER_SCHEMA_VERSION.to_string(),
        entry_type: entry_type.to_string(),
        amount_credits,
        reservation_id: reservation_id.map(ToOwned::to_owned),
        reason: reason.to_string(),
        created_at: now,
    })
}

async fn insert_customer_audit_event(
    transaction: &Transaction<'_>,
    event: NewCustomerAuditEvent<'_>,
) -> Result<String, SessionError> {
    let event_id = format!("caudit_{}", Uuid::new_v4());
    transaction
        .execute(
            "INSERT INTO customer_audit_events (customer_audit_event_id, organization_id, project_id, schema_version, actor_type, actor_id, event_type, entity_type, entity_id, summary, metadata_json, occurred_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            &[
                &event_id,
                &event.organization_id,
                &event.project_id,
                &CUSTOMER_AUDIT_SCHEMA_VERSION,
                &event.actor_type,
                &event.actor_id,
                &event.event_type,
                &event.entity_type,
                &event.entity_id,
                &event.summary,
                &event.metadata_json,
                &Utc::now().to_rfc3339(),
            ],
        )
        .await?;
    Ok(event_id)
}

async fn load_project_quota(
    transaction: &Transaction<'_>,
    project_id: &str,
) -> Result<ProjectQuotaRecord, SessionError> {
    let row = transaction
        .query_one(
            "SELECT project_id, max_active_reservations, max_reserved_gpu_seconds, max_reservation_ttl_seconds, updated_at FROM project_quotas WHERE project_id = $1",
            &[&project_id],
        )
        .await?;
    Ok(ProjectQuotaRecord {
        project_id: row.get("project_id"),
        max_active_reservations: from_i32_to_u32(row.get("max_active_reservations"))?,
        max_reserved_gpu_seconds: from_i64_to_u64(row.get("max_reserved_gpu_seconds"))?,
        max_reservation_ttl_seconds: from_i32_to_u32(row.get("max_reservation_ttl_seconds"))?,
        updated_at: row.get("updated_at"),
    })
}

async fn load_customer_api_key(
    transaction: &Transaction<'_>,
    api_key_id: &str,
) -> Result<CustomerApiKeyRecord, SessionError> {
    let row = transaction
        .query_one(
            "SELECT api_key_id, organization_id, project_id, schema_version, key_prefix, status, scopes_json, created_at, last_used_at, expires_at, revoked_at FROM customer_api_keys WHERE api_key_id = $1",
            &[&api_key_id],
        )
        .await?;
    api_key_from_row(row)
}

async fn load_reservation(
    transaction: &Transaction<'_>,
    reservation_id: &str,
) -> Result<MarketplaceReservationRecord, SessionError> {
    let row = transaction
        .query_opt(
            &format!(
                "{} WHERE reservation_id = $1 FOR UPDATE",
                reservation_select_columns()
            ),
            &[&reservation_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("reservation not found".to_string()))?;
    reservation_from_row(row)
}
fn organization_from_row(row: Row) -> OrganizationRecord {
    OrganizationRecord {
        organization_id: row.get("organization_id"),
        schema_version: row.get("schema_version"),
        display_name: row.get("display_name"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn api_key_from_row(row: Row) -> Result<CustomerApiKeyRecord, SessionError> {
    let scopes_json: String = row.get("scopes_json");
    Ok(CustomerApiKeyRecord {
        api_key_id: row.get("api_key_id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        schema_version: row.get("schema_version"),
        key_prefix: row.get("key_prefix"),
        status: row.get("status"),
        scopes: serde_json::from_str(&scopes_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        created_at: row.get("created_at"),
        last_used_at: row.get("last_used_at"),
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
    })
}

fn reservation_from_row(row: Row) -> Result<MarketplaceReservationRecord, SessionError> {
    let reason_codes_json: String = row.get("reason_codes_json");
    Ok(MarketplaceReservationRecord {
        reservation_id: row.get("reservation_id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        listing_id: row.get("listing_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        schema_version: row.get("schema_version"),
        workload_type: row.get("workload_type"),
        gpu_uuid: row.get("gpu_uuid"),
        status: row.get("status"),
        starts_at: row.get("starts_at"),
        expires_at: row.get("expires_at"),
        cancelled_at: row.get("cancelled_at"),
        reserved_gpu_seconds: from_i64_to_u64(row.get("reserved_gpu_seconds"))?,
        reason_codes: serde_json::from_str(&reason_codes_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn customer_audit_from_row(row: Row) -> Result<CustomerAuditEventRecord, SessionError> {
    let metadata_json: String = row.get("metadata_json");
    Ok(CustomerAuditEventRecord {
        customer_audit_event_id: row.get("customer_audit_event_id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        schema_version: row.get("schema_version"),
        actor_type: row.get("actor_type"),
        actor_id: row.get("actor_id"),
        event_type: row.get("event_type"),
        entity_type: row.get("entity_type"),
        entity_id: row.get("entity_id"),
        summary: row.get("summary"),
        metadata: serde_json::from_str(&metadata_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        occurred_at: row.get("occurred_at"),
    })
}

fn idempotency_from_row(row: Row) -> IdempotencyRecord {
    IdempotencyRecord {
        request_hash: row.get("request_hash"),
        status_code: row.get::<_, i32>("status_code") as u16,
        response_json: row.get("response_json"),
    }
}

fn organization_select_columns() -> &'static str {
    "SELECT organization_id, schema_version, display_name, status, created_at, updated_at FROM organizations"
}

fn reservation_select_columns() -> &'static str {
    "SELECT reservation_id, organization_id, project_id, listing_id, provider_id, device_id, session_id, schema_version, workload_type, gpu_uuid, status, starts_at, expires_at, cancelled_at, reserved_gpu_seconds, reason_codes_json, created_at, updated_at FROM marketplace_reservations"
}

fn customer_audit_select_columns() -> &'static str {
    "SELECT customer_audit_event_id, organization_id, project_id, schema_version, actor_type, actor_id, event_type, entity_type, entity_id, summary, metadata_json, occurred_at FROM customer_audit_events"
}

fn validate_customer_user_request(request: &CreateCustomerUserRequest) -> Result<(), SessionError> {
    if let Some(email) = request.email.as_deref() {
        validate_email(email)?;
    }
    if let Some(status) = request.status.as_deref()
        && !matches!(status, "active" | "disabled")
    {
        return Err(SessionError::Invalid(
            "customer user status must be active or disabled".to_string(),
        ));
    }
    Ok(())
}

fn validate_organization_request(request: &CreateOrganizationRequest) -> Result<(), SessionError> {
    validate_display_name("organization display_name", &request.display_name)?;
    if let Some(owner_user_id) = request.owner_user_id.as_deref() {
        validate_id("owner_user_id", owner_user_id, 128)?;
    }
    Ok(())
}

fn validate_project_request(request: &CreateProjectRequest) -> Result<(), SessionError> {
    validate_display_name("project display_name", &request.display_name)
}

fn validate_quota_request(request: &UpsertProjectQuotaRequest) -> Result<(), SessionError> {
    if request.max_active_reservations == 0 || request.max_active_reservations > 100 {
        return Err(SessionError::Invalid(
            "max_active_reservations must be between 1 and 100".to_string(),
        ));
    }
    if request.max_reserved_gpu_seconds == 0 {
        return Err(SessionError::Invalid(
            "max_reserved_gpu_seconds must be greater than zero".to_string(),
        ));
    }
    if request.max_reservation_ttl_seconds == 0
        || request.max_reservation_ttl_seconds > MAX_RESERVATION_TTL_SECONDS
    {
        return Err(SessionError::Invalid(
            "max_reservation_ttl_seconds is outside allowed range".to_string(),
        ));
    }
    Ok(())
}

fn validate_credit_request(request: &GrantCustomerCreditsRequest) -> Result<(), SessionError> {
    if request.amount_credits == 0 {
        return Err(SessionError::Invalid(
            "amount_credits must be non-zero".to_string(),
        ));
    }
    if !is_bounded_ascii(&request.reason, 256) {
        return Err(SessionError::Invalid(
            "credit reason must be printable ASCII".to_string(),
        ));
    }
    Ok(())
}

fn validate_reservation_request(request: &CreateReservationRequest) -> Result<(), SessionError> {
    validate_id("listing_id", &request.listing_id, 160)?;
    if request.duration_seconds == 0 || request.duration_seconds > MAX_RESERVATION_TTL_SECONDS {
        return Err(SessionError::Invalid(
            "reservation duration_seconds is outside allowed range".to_string(),
        ));
    }
    if let Some(starts_at) = request.starts_at.as_deref() {
        parse_timestamp("starts_at", starts_at)?;
    }
    if let Some(workload_type) = request.workload_type.as_deref() {
        validate_id("workload_type", workload_type, 96)?;
    }
    Ok(())
}

fn normalized_scopes(scopes: &[String]) -> Result<Vec<String>, SessionError> {
    let mut normalized = if scopes.is_empty() {
        DEFAULT_CUSTOMER_SCOPES
            .iter()
            .map(|scope| (*scope).to_string())
            .collect::<Vec<_>>()
    } else {
        scopes.to_vec()
    };
    for scope in &normalized {
        validate_id("scope", scope, 64)?;
        if !ALLOWED_CUSTOMER_SCOPES
            .iter()
            .any(|allowed| allowed == scope)
        {
            return Err(SessionError::Invalid(
                "customer API key scope is not supported".to_string(),
            ));
        }
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn require_customer_scope(auth: &CustomerApiKeyAuth, scope: &str) -> Result<(), SessionError> {
    if auth.scopes.iter().any(|candidate| candidate == scope) {
        Ok(())
    } else {
        Err(SessionError::Unauthorized)
    }
}

fn reservation_start_time(
    starts_at: Option<&str>,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, SessionError> {
    let Some(starts_at) = starts_at else {
        return Ok(now);
    };
    let parsed = parse_timestamp("starts_at", starts_at)?;
    if parsed + Duration::seconds(30) < now {
        return Err(SessionError::Invalid(
            "reservation starts_at cannot be in the past".to_string(),
        ));
    }
    Ok(parsed)
}

fn validate_future_timestamp(label: &str, value: &str) -> Result<(), SessionError> {
    let parsed = parse_timestamp(label, value)?;
    if parsed <= Utc::now() {
        return Err(SessionError::Invalid(format!(
            "{label} must be in the future"
        )));
    }
    Ok(())
}

fn parse_timestamp(label: &str, value: &str) -> Result<DateTime<Utc>, SessionError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| SessionError::Invalid(format!("invalid {label}: {error}")))
}

fn validate_email(email: &str) -> Result<(), SessionError> {
    let valid = is_bounded_ascii(email, 254)
        && email.contains('@')
        && !email.contains(' ')
        && !email.contains("..");
    if valid {
        Ok(())
    } else {
        Err(SessionError::Invalid(
            "email must be a short printable ASCII address".to_string(),
        ))
    }
}

fn validate_display_name(label: &str, value: &str) -> Result<(), SessionError> {
    if is_bounded_ascii(value, 120) {
        Ok(())
    } else {
        Err(SessionError::Invalid(format!(
            "{label} must be short printable ASCII"
        )))
    }
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

fn to_i64(value: u64) -> Result<i64, SessionError> {
    i64::try_from(value).map_err(|_| SessionError::Invalid("quantity exceeds i64".to_string()))
}

fn from_i64_to_u64(value: i64) -> Result<u64, SessionError> {
    u64::try_from(value)
        .map_err(|_| SessionError::Database(DbError::new("negative customer quantity")))
}

fn from_i64_to_u32(value: i64) -> Result<u32, SessionError> {
    u32::try_from(value)
        .map_err(|_| SessionError::Database(DbError::new("customer count overflow")))
}

fn from_i32_to_u32(value: i32) -> Result<u32, SessionError> {
    u32::try_from(value)
        .map_err(|_| SessionError::Database(DbError::new("negative customer count")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_auth(scopes: Vec<&str>) -> CustomerApiKeyAuth {
        CustomerApiKeyAuth {
            api_key_id: "cak_1".to_string(),
            organization_id: "org_1".to_string(),
            project_id: Some("project_1".to_string()),
            scopes: scopes.into_iter().map(ToOwned::to_owned).collect(),
        }
    }

    #[test]
    fn customer_scope_validation_defaults_and_deduplicates() {
        assert_eq!(
            normalized_scopes(&[]).unwrap(),
            vec![
                "billing:read".to_string(),
                "billing:write".to_string(),
                "reservations:read".to_string(),
                "reservations:write".to_string(),
                "usage:read".to_string(),
            ]
        );
        let scopes = normalized_scopes(&[
            "usage:read".to_string(),
            "usage:read".to_string(),
            "reservations:read".to_string(),
        ])
        .unwrap();
        assert_eq!(
            scopes,
            vec!["reservations:read".to_string(), "usage:read".to_string()]
        );
        assert_eq!(
            normalized_scopes(&["billing:write".to_string()]).unwrap(),
            vec!["billing:write".to_string()]
        );
    }

    #[test]
    fn customer_scope_authorization_is_explicit() {
        let auth = test_auth(vec!["reservations:read"]);
        assert!(require_customer_scope(&auth, "reservations:read").is_ok());
        assert!(require_customer_scope(&auth, "reservations:write").is_err());
    }

    #[test]
    fn reservation_request_requires_duration_and_listing() {
        let valid = CreateReservationRequest {
            listing_id: "listing_1".to_string(),
            duration_seconds: 60,
            starts_at: None,
            workload_type: Some("llm_batch_inference".to_string()),
        };
        assert!(validate_reservation_request(&valid).is_ok());
        let mut invalid = valid;
        invalid.duration_seconds = 0;
        assert!(validate_reservation_request(&invalid).is_err());
    }
    #[tokio::test]
    #[ignore]
    async fn postgres_customer_reservation_flow_persists_usage_and_cancellation() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_customer_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let user = db
            .create_customer_user(
                "req_customer_user",
                &CreateCustomerUserRequest {
                    email: Some("customer@example.test".to_string()),
                    status: None,
                },
            )
            .await
            .unwrap()
            .user;
        let organization = db
            .create_organization(
                "req_org",
                &CreateOrganizationRequest {
                    display_name: "Example Customer".to_string(),
                    owner_user_id: Some(user.user_id),
                },
            )
            .await
            .unwrap()
            .organization;
        let project = db
            .create_project(
                "req_project",
                &organization.organization_id,
                &CreateProjectRequest {
                    display_name: "Inference".to_string(),
                },
            )
            .await
            .unwrap()
            .project;
        db.upsert_project_quota(
            "req_quota",
            &project.project_id,
            &UpsertProjectQuotaRequest {
                max_active_reservations: 2,
                max_reserved_gpu_seconds: 3600,
                max_reservation_ttl_seconds: 3600,
            },
        )
        .await
        .unwrap();
        let key = db
            .create_customer_api_key(
                "req_key",
                &project.project_id,
                &CreateCustomerApiKeyRequest::default(),
            )
            .await
            .unwrap();
        let auth = db
            .authorize_customer_api_key(&key.token, Some(&project.project_id))
            .await
            .unwrap();

        seed_reservable_listing(&db).await;
        let request = CreateReservationRequest {
            listing_id: "listing_customer_flow".to_string(),
            duration_seconds: 60,
            starts_at: None,
            workload_type: Some("llm_batch_inference".to_string()),
        };
        let first = db
            .create_marketplace_reservation_idempotently(CreateReservationCommand {
                request_id: "req_reservation".to_string(),
                scope: format!(
                    "POST /v1/customer/projects/{}/reservations",
                    project.project_id
                ),
                idempotency_key: "customer-reservation-1".to_string(),
                request_hash: "reservation-hash-1".to_string(),
                auth: auth.clone(),
                project_id: project.project_id.clone(),
                request: request.clone(),
            })
            .await
            .unwrap();
        let CreateReservationOutcome::Response(first) = first else {
            panic!("first reservation must create a response");
        };
        let response: MarketplaceReservationResponse =
            serde_json::from_str(&first.response_json).unwrap();
        assert_eq!(response.reservation.status, "reserved");
        assert_eq!(response.reservation.project_id, project.project_id);

        let usage = db
            .customer_project_usage("req_usage", &auth, &project.project_id)
            .await
            .unwrap()
            .usage;
        assert_eq!(usage.active_reservations, 1);
        assert_eq!(usage.reserved_gpu_seconds, 60);

        let cancelled = db
            .cancel_marketplace_reservation(
                "req_cancel",
                &auth,
                &response.reservation.reservation_id,
                &CancelReservationRequest {
                    reason: Some("customer_request".to_string()),
                },
            )
            .await
            .unwrap();
        assert_eq!(cancelled.reservation.status, "cancelled");
        let usage = db
            .customer_project_usage("req_usage_after", &auth, &project.project_id)
            .await
            .unwrap()
            .usage;
        assert_eq!(usage.active_reservations, 0);
        assert_eq!(usage.cancelled_reservations, 1);

        db.drop_schema_for_test().await.unwrap();
    }

    async fn seed_reservable_listing(db: &Database) {
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        let expires_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
        client
            .batch_execute(&format!(
                "INSERT INTO providers (provider_id, display_name, status, created_at, updated_at) VALUES ('provider_customer_flow', 'Provider Customer Flow', 'verified', '{now}', '{now}');
                 INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ('device_customer_flow', 'provider_customer_flow', 'machine_customer_flow', 'active', '{now}', '{now}');
                 INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, last_seen_at, expires_at, hardware_fingerprint, updated_at) VALUES ('session_customer_flow', 'provider_customer_flow', 'device_customer_flow', 'online', 1, '{now}', '{now}', '{expires_at}', 'fp_customer_flow', '{now}');
                 INSERT INTO workload_policies (policy_id, policy_version, schema_version, workload_type, display_name, requirements_json, status, created_at, updated_at) VALUES ('policy_customer_flow', 'v1', 'burd-workload-policy-v1', 'llm_batch_inference', 'Customer Flow Policy', '{{}}', 'active', '{now}', '{now}');
                 INSERT INTO marketplace_listings (listing_id, provider_id, provider_display_name, device_id, session_id, schema_version, engine_version, status, current_status, workload_type, policy_id, policy_version, gpu_uuid, gpu_verified, gpu_verification_source, vram_total_mib, vram_verified, vram_verification_source, region, region_source, trust_score, risk_score, reliability_score, verification_status, proof_freshness_status, last_verified_at, remote_network_score, effective_network_score, regional_reachability_json, benchmark_profile_id, benchmark_profile_version, benchmark_status, benchmark_completed_at, price_source, availability_window_json, active_lease_count, reason_codes_json, source_hash, published_at, updated_at) VALUES ('listing_customer_flow', 'provider_customer_flow', 'Provider Customer Flow', 'device_customer_flow', 'session_customer_flow', 'burd-marketplace-listing-v1', 'burd-marketplace-engine-v1', 'published', 'available', 'llm_batch_inference', 'policy_customer_flow', 'v1', 'GPU-customer-flow', TRUE, 'backend_proof_and_benchmark', 24576, TRUE, 'backend_telemetry_bound_to_verified_gpu', 'us-east', 'regional_probe', 91.0, 3.0, 96.0, 'verified', 'freshness_backend_timestamp_present', '{now}', 92.0, 93.0, '[]', 'bench_profile_customer_flow', 'v1', 'succeeded', '{now}', 'not_configured_bn16', '{{\"reservations_enabled\":true}}', 0, '[]', 'source_hash_customer_flow', '{now}', '{now}');"
            ))
            .await
            .unwrap();
    }
}
