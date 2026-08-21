use crate::customer::{
    DEFAULT_MAX_ACTIVE_RESERVATIONS, DEFAULT_MAX_RESERVATION_TTL_SECONDS,
    DEFAULT_MAX_RESERVED_GPU_SECONDS, NewCustomerAuditEvent, insert_customer_audit_event,
    load_customer_api_key, normalized_scopes,
};
use crate::db::Database;
use crate::human_auth::HumanSessionAuth;
use crate::remote_session::SessionError;
use burd_protocol::{
    AddOrganizationMemberRequest, CUSTOMER_API_KEY_SCHEMA_VERSION,
    CUSTOMER_ORGANIZATION_SCHEMA_VERSION, CUSTOMER_PROJECT_SCHEMA_VERSION,
    CreateCustomerApiKeyRequest, CreateCustomerApiKeyResponse, CreateOrganizationRequest,
    CreateProjectRequest, ListCustomerApiKeysResponse, ListOrganizationMembersResponse,
    OrganizationMemberResponse, OrganizationMembershipRecord, OrganizationRecord,
    OrganizationResponse, ProjectRecord, ProjectResponse, RevokeCustomerApiKeyResponse,
    UpdateOrganizationMemberRequest, random_token, sha256_hex,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationPermission {
    OrganizationRead,
    OrganizationManage,
    MembersRead,
    MembersManage,
    ProjectsRead,
    ProjectsManage,
    ApiKeysRead,
    ApiKeysManage,
    BillingRead,
    BillingManage,
    WorkloadsRead,
    WorkloadsWrite,
    ReservationsRead,
    ReservationsManage,
    ArtifactsRead,
    ArtifactsWrite,
    UsageRead,
    AuditRead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganizationRole {
    Owner,
    Admin,
    BillingAdmin,
    Developer,
    Viewer,
}

impl OrganizationRole {
    pub fn parse(value: &str) -> Result<Self, SessionError> {
        match value {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "billing_admin" => Ok(Self::BillingAdmin),
            "developer" => Ok(Self::Developer),
            "viewer" => Ok(Self::Viewer),
            _ => Err(SessionError::Unauthorized),
        }
    }

    fn allows(self, permission: OrganizationPermission) -> bool {
        use OrganizationPermission as P;
        match self {
            Self::Owner => true,
            Self::Admin => !matches!(permission, P::OrganizationManage | P::BillingManage),
            Self::BillingAdmin => matches!(
                permission,
                P::OrganizationRead | P::BillingRead | P::BillingManage | P::UsageRead
            ),
            Self::Developer => matches!(
                permission,
                P::OrganizationRead
                    | P::ProjectsRead
                    | P::ApiKeysRead
                    | P::ApiKeysManage
                    | P::WorkloadsRead
                    | P::WorkloadsWrite
                    | P::ReservationsRead
                    | P::ReservationsManage
                    | P::ArtifactsRead
                    | P::ArtifactsWrite
                    | P::UsageRead
            ),
            Self::Viewer => matches!(
                permission,
                P::OrganizationRead
                    | P::ProjectsRead
                    | P::ApiKeysRead
                    | P::WorkloadsRead
                    | P::ReservationsRead
                    | P::ArtifactsRead
                    | P::UsageRead
                    | P::AuditRead
                    | P::BillingRead
            ),
        }
    }
}

pub async fn authorize_organization_member(
    transaction: &Transaction<'_>,
    auth: &HumanSessionAuth,
    organization_id: &str,
    permission: OrganizationPermission,
) -> Result<OrganizationRole, SessionError> {
    // Organization-wide mutations (including membership changes and their audit FKs) use a
    // single lock order: organization first, membership second. This prevents revoke/downgrade
    // races from deadlocking with a concurrently authorized mutation.
    transaction
        .query_opt(
            "SELECT organization_id FROM organizations WHERE organization_id = $1 FOR UPDATE",
            &[&organization_id],
        )
        .await?
        .ok_or(SessionError::Unauthorized)?;
    let row = transaction.query_opt("SELECT ou.role, ou.status, u.status AS user_status, o.status AS organization_status FROM organization_users ou JOIN users u ON u.user_id = ou.user_id JOIN organizations o ON o.organization_id = ou.organization_id WHERE ou.organization_id = $1 AND ou.user_id = $2 FOR UPDATE OF ou", &[&organization_id, &auth.user_id]).await?.ok_or(SessionError::Unauthorized)?;
    let role = OrganizationRole::parse(row.get::<_, String>("role").as_str())?;
    if row.get::<_, String>("status") != "active"
        || row.get::<_, String>("user_status") != "active"
        || row.get::<_, String>("organization_status") != "active"
        || !role.allows(permission)
    {
        return Err(SessionError::Unauthorized);
    }
    Ok(role)
}

impl Database {
    pub async fn create_human_organization(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
        request: &CreateOrganizationRequest,
    ) -> Result<OrganizationResponse, SessionError> {
        if request.display_name.trim().is_empty()
            || request.display_name.len() > 160
            || request.owner_user_id.is_some()
        {
            return Err(SessionError::Invalid(
                "human organization creation does not accept owner_user_id".to_string(),
            ));
        }
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .query_one(
                "SELECT user_id FROM users WHERE user_id = $1 AND status = 'active' FOR UPDATE",
                &[&auth.user_id],
            )
            .await
            .map_err(|_| SessionError::Unauthorized)?;
        let now = Utc::now().to_rfc3339();
        let organization = OrganizationRecord {
            organization_id: format!("org_{}", Uuid::new_v4()),
            schema_version: CUSTOMER_ORGANIZATION_SCHEMA_VERSION.to_string(),
            display_name: request.display_name.trim().to_string(),
            status: "active".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        transaction.execute("INSERT INTO organizations (organization_id, schema_version, display_name, status, created_at, updated_at) VALUES ($1,$2,$3,'active',$4,$4)", &[&organization.organization_id,&organization.schema_version,&organization.display_name,&now]).await?;
        transaction.execute("INSERT INTO organization_users (organization_id,user_id,role,status,created_at,updated_at) VALUES ($1,$2,'owner','active',$3,$3)", &[&organization.organization_id,&auth.user_id,&now]).await?;
        human_customer_audit(
            &transaction,
            &organization.organization_id,
            None,
            auth,
            "organization.created",
            "organization",
            &organization.organization_id,
            "human-created organization and owner membership",
        )
        .await?;
        transaction.commit().await?;
        Ok(OrganizationResponse {
            request_id: request_id.to_string(),
            organization: organization.clone(),
            membership: Some(OrganizationMembershipRecord {
                organization_id: organization.organization_id,
                user_id: auth.user_id.clone(),
                role: "owner".to_string(),
                status: "active".to_string(),
                created_at: now.clone(),
                updated_at: now,
            }),
        })
    }

    pub async fn list_organization_members(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
        organization_id: &str,
    ) -> Result<ListOrganizationMembersResponse, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        authorize_organization_member(
            &transaction,
            auth,
            organization_id,
            OrganizationPermission::MembersRead,
        )
        .await?;
        let members = transaction.query("SELECT organization_id,user_id,role,status,created_at,updated_at FROM organization_users WHERE organization_id=$1 ORDER BY created_at,user_id", &[&organization_id]).await?.into_iter().map(member_from_row).collect();
        transaction.commit().await?;
        Ok(ListOrganizationMembersResponse {
            request_id: request_id.to_string(),
            members,
        })
    }

    pub async fn add_organization_member(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
        organization_id: &str,
        request: &AddOrganizationMemberRequest,
    ) -> Result<OrganizationMemberResponse, SessionError> {
        let target_role = OrganizationRole::parse(&request.role)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let actor_role = authorize_organization_member(
            &transaction,
            auth,
            organization_id,
            OrganizationPermission::MembersManage,
        )
        .await?;
        enforce_role_management(actor_role, target_role)?;
        transaction
            .query_one(
                "SELECT user_id FROM users WHERE user_id=$1 AND status='active'",
                &[&request.user_id],
            )
            .await
            .map_err(|_| {
                SessionError::Invalid("member user must exist and be active".to_string())
            })?;
        let now = Utc::now().to_rfc3339();
        transaction.execute("INSERT INTO organization_users (organization_id,user_id,role,status,created_at,updated_at) VALUES ($1,$2,$3,'active',$4,$4) ON CONFLICT (organization_id,user_id) DO NOTHING", &[&organization_id,&request.user_id,&request.role,&now]).await?;
        let row=transaction.query_one("SELECT organization_id,user_id,role,status,created_at,updated_at FROM organization_users WHERE organization_id=$1 AND user_id=$2", &[&organization_id,&request.user_id]).await?;
        let member = member_from_row(row);
        human_customer_audit(
            &transaction,
            organization_id,
            None,
            auth,
            "organization_member.added",
            "user",
            &request.user_id,
            "organization member added",
        )
        .await?;
        transaction.commit().await?;
        Ok(OrganizationMemberResponse {
            request_id: request_id.to_string(),
            member,
        })
    }

    pub async fn update_organization_member(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
        organization_id: &str,
        user_id: &str,
        request: &UpdateOrganizationMemberRequest,
    ) -> Result<OrganizationMemberResponse, SessionError> {
        if request.role.is_none() && request.status.is_none() {
            return Err(SessionError::Invalid(
                "role or status is required".to_string(),
            ));
        }
        if request
            .status
            .as_deref()
            .is_some_and(|s| !matches!(s, "active" | "inactive"))
        {
            return Err(SessionError::Invalid(
                "membership status is invalid".to_string(),
            ));
        }
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .query_one(
                "SELECT organization_id FROM organizations WHERE organization_id=$1 FOR UPDATE",
                &[&organization_id],
            )
            .await?;
        let actor_role = authorize_organization_member(
            &transaction,
            auth,
            organization_id,
            OrganizationPermission::MembersManage,
        )
        .await?;
        let current=transaction.query_opt("SELECT role,status FROM organization_users WHERE organization_id=$1 AND user_id=$2 FOR UPDATE", &[&organization_id,&user_id]).await?.ok_or_else(||SessionError::NotFound("organization member not found".to_string()))?;
        let current_role = OrganizationRole::parse(current.get::<_, String>("role").as_str())?;
        let next_role = match request.role.as_deref() {
            Some(role) => OrganizationRole::parse(role)?,
            None => current_role,
        };
        enforce_role_management(actor_role, current_role)?;
        enforce_role_management(actor_role, next_role)?;
        let next_status = request
            .status
            .as_deref()
            .unwrap_or(current.get::<_, String>("status").as_str())
            .to_string();
        if current_role == OrganizationRole::Owner
            && (next_role != OrganizationRole::Owner || next_status != "active")
        {
            require_another_active_owner(&transaction, organization_id, user_id).await?;
        }
        let role = request.role.clone().unwrap_or_else(|| current.get("role"));
        let now = Utc::now().to_rfc3339();
        transaction.execute("UPDATE organization_users SET role=$1,status=$2,updated_at=$3 WHERE organization_id=$4 AND user_id=$5", &[&role,&next_status,&now,&organization_id,&user_id]).await?;
        let member=member_from_row(transaction.query_one("SELECT organization_id,user_id,role,status,created_at,updated_at FROM organization_users WHERE organization_id=$1 AND user_id=$2", &[&organization_id,&user_id]).await?);
        human_customer_audit(
            &transaction,
            organization_id,
            None,
            auth,
            "organization_member.updated",
            "user",
            user_id,
            "organization member updated",
        )
        .await?;
        transaction.commit().await?;
        Ok(OrganizationMemberResponse {
            request_id: request_id.to_string(),
            member,
        })
    }

    pub async fn remove_organization_member(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
        organization_id: &str,
        user_id: &str,
    ) -> Result<(), SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .query_one(
                "SELECT organization_id FROM organizations WHERE organization_id=$1 FOR UPDATE",
                &[&organization_id],
            )
            .await?;
        let actor_role = authorize_organization_member(
            &transaction,
            auth,
            organization_id,
            OrganizationPermission::MembersManage,
        )
        .await?;
        let row=transaction.query_opt("SELECT role,status FROM organization_users WHERE organization_id=$1 AND user_id=$2 FOR UPDATE", &[&organization_id,&user_id]).await?.ok_or_else(||SessionError::NotFound("organization member not found".to_string()))?;
        let target_role = OrganizationRole::parse(row.get::<_, String>("role").as_str())?;
        enforce_role_management(actor_role, target_role)?;
        if target_role == OrganizationRole::Owner && row.get::<_, String>("status") == "active" {
            require_another_active_owner(&transaction, organization_id, user_id).await?;
        }
        transaction
            .execute(
                "DELETE FROM organization_users WHERE organization_id=$1 AND user_id=$2",
                &[&organization_id, &user_id],
            )
            .await?;
        human_customer_audit(
            &transaction,
            organization_id,
            None,
            auth,
            "organization_member.removed",
            "user",
            user_id,
            "organization member removed",
        )
        .await?;
        transaction.commit().await?;
        let _ = request_id;
        Ok(())
    }

    pub async fn create_human_project(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
        organization_id: &str,
        request: &CreateProjectRequest,
    ) -> Result<ProjectResponse, SessionError> {
        if request.display_name.trim().is_empty() || request.display_name.len() > 160 {
            return Err(SessionError::Invalid(
                "project display_name is invalid".to_string(),
            ));
        }
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        authorize_organization_member(
            &transaction,
            auth,
            organization_id,
            OrganizationPermission::ProjectsManage,
        )
        .await?;
        let now = Utc::now().to_rfc3339();
        let project = ProjectRecord {
            project_id: format!("project_{}", Uuid::new_v4()),
            organization_id: organization_id.to_string(),
            schema_version: CUSTOMER_PROJECT_SCHEMA_VERSION.to_string(),
            display_name: request.display_name.trim().to_string(),
            status: "active".to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        transaction.execute("INSERT INTO projects(project_id,organization_id,schema_version,display_name,status,created_at,updated_at) VALUES($1,$2,$3,$4,'active',$5,$5)",&[&project.project_id,&project.organization_id,&project.schema_version,&project.display_name,&now]).await?;
        transaction.execute("INSERT INTO project_quotas(project_id,max_active_reservations,max_reserved_gpu_seconds,max_reservation_ttl_seconds,updated_at) VALUES($1,$2,$3,$4,$5)",&[&project.project_id,&(DEFAULT_MAX_ACTIVE_RESERVATIONS as i32),&(DEFAULT_MAX_RESERVED_GPU_SECONDS as i64),&(DEFAULT_MAX_RESERVATION_TTL_SECONDS as i32),&now]).await?;
        human_customer_audit(
            &transaction,
            organization_id,
            Some(project.project_id.clone()),
            auth,
            "project.created",
            "project",
            &project.project_id,
            "project created by human user",
        )
        .await?;
        transaction.commit().await?;
        Ok(ProjectResponse {
            request_id: request_id.to_string(),
            project,
        })
    }

    pub async fn create_human_customer_api_key(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
        project_id: &str,
        request: &CreateCustomerApiKeyRequest,
    ) -> Result<CreateCustomerApiKeyResponse, SessionError> {
        let scopes = normalized_scopes(&request.scopes)?;
        if let Some(expires_at) = request.expires_at.as_deref() {
            let expires_at = chrono::DateTime::parse_from_rfc3339(expires_at)
                .map_err(|_| SessionError::Invalid("expires_at must be RFC3339".to_string()))?
                .with_timezone(&Utc);
            if expires_at <= Utc::now() {
                return Err(SessionError::Invalid(
                    "expires_at must be in the future".to_string(),
                ));
            }
        }
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let organization_id = project_organization(&transaction, project_id).await?;
        authorize_organization_member(
            &transaction,
            auth,
            &organization_id,
            OrganizationPermission::ApiKeysManage,
        )
        .await?;
        let token = random_token("burd_customer").map_err(SessionError::Invalid)?;
        let hash = sha256_hex(token.as_bytes());
        let prefix = token.chars().take(24).collect::<String>();
        let id = format!("cak_{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        let scopes_json =
            serde_json::to_string(&scopes).map_err(|e| SessionError::Invalid(e.to_string()))?;
        transaction.execute("INSERT INTO customer_api_keys(api_key_id,organization_id,project_id,schema_version,key_prefix,key_hash,status,scopes_json,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,'active',$7,$8,$9)",&[&id,&organization_id,&Some(project_id.to_string()),&CUSTOMER_API_KEY_SCHEMA_VERSION,&prefix,&hash,&scopes_json,&now,&request.expires_at]).await?;
        let api_key = load_customer_api_key(&transaction, &id).await?;
        human_customer_audit(
            &transaction,
            &organization_id,
            Some(project_id.to_string()),
            auth,
            "customer_api_key.created",
            "customer_api_key",
            &id,
            "project API key created; plaintext returned once",
        )
        .await?;
        transaction.commit().await?;
        Ok(CreateCustomerApiKeyResponse {
            request_id: request_id.to_string(),
            api_key,
            token,
        })
    }

    pub async fn list_human_customer_api_keys(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
        project_id: &str,
    ) -> Result<ListCustomerApiKeysResponse, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let org = project_organization(&transaction, project_id).await?;
        authorize_organization_member(
            &transaction,
            auth,
            &org,
            OrganizationPermission::ApiKeysRead,
        )
        .await?;
        let mut keys = Vec::new();
        for row in transaction.query("SELECT api_key_id FROM customer_api_keys WHERE project_id=$1 ORDER BY created_at DESC", &[&project_id]).await?{keys.push(load_customer_api_key(&transaction,row.get::<_,String>("api_key_id").as_str()).await?);}
        transaction.commit().await?;
        Ok(ListCustomerApiKeysResponse {
            request_id: request_id.to_string(),
            api_keys: keys,
        })
    }

    pub async fn revoke_human_customer_api_key(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
        project_id: &str,
        api_key_id: &str,
    ) -> Result<RevokeCustomerApiKeyResponse, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let org = project_organization(&transaction, project_id).await?;
        authorize_organization_member(
            &transaction,
            auth,
            &org,
            OrganizationPermission::ApiKeysManage,
        )
        .await?;
        let now = Utc::now().to_rfc3339();
        let count=transaction.execute("UPDATE customer_api_keys SET status='revoked',revoked_at=COALESCE(revoked_at,$1) WHERE api_key_id=$2 AND project_id=$3 AND status IN ('active','revoked')", &[&now,&api_key_id,&project_id]).await?;
        if count == 0 {
            return Err(SessionError::NotFound(
                "customer API key not found".to_string(),
            ));
        }
        let api_key = load_customer_api_key(&transaction, api_key_id).await?;
        human_customer_audit(
            &transaction,
            &org,
            Some(project_id.to_string()),
            auth,
            "customer_api_key.revoked",
            "customer_api_key",
            api_key_id,
            "project API key revoked",
        )
        .await?;
        transaction.commit().await?;
        Ok(RevokeCustomerApiKeyResponse {
            request_id: request_id.to_string(),
            api_key,
        })
    }
}

fn enforce_role_management(
    actor: OrganizationRole,
    target: OrganizationRole,
) -> Result<(), SessionError> {
    if target == OrganizationRole::Owner && actor != OrganizationRole::Owner {
        return Err(SessionError::Unauthorized);
    }
    Ok(())
}
async fn require_another_active_owner(
    tx: &Transaction<'_>,
    org: &str,
    excluded: &str,
) -> Result<(), SessionError> {
    let count:i64=tx.query_one("SELECT COUNT(*) AS count FROM organization_users WHERE organization_id=$1 AND role='owner' AND status='active' AND user_id<>$2", &[&org,&excluded]).await?.get("count");
    if count < 1 {
        return Err(SessionError::Conflict(
            "organization must retain an active owner".to_string(),
        ));
    }
    Ok(())
}
async fn project_organization(tx: &Transaction<'_>, project: &str) -> Result<String, SessionError> {
    tx.query_opt(
        "SELECT organization_id FROM projects WHERE project_id=$1 AND status='active'",
        &[&project],
    )
    .await?
    .map(|r| r.get("organization_id"))
    .ok_or_else(|| SessionError::NotFound("project not found".to_string()))
}
fn member_from_row(row: Row) -> OrganizationMembershipRecord {
    OrganizationMembershipRecord {
        organization_id: row.get("organization_id"),
        user_id: row.get("user_id"),
        role: row.get("role"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
#[allow(
    clippy::too_many_arguments,
    reason = "audit authority fields stay explicit at the security boundary"
)]
async fn human_customer_audit(
    tx: &Transaction<'_>,
    org: &str,
    project: Option<String>,
    auth: &HumanSessionAuth,
    event: &str,
    entity_type: &str,
    entity_id: &str,
    summary: &str,
) -> Result<(), SessionError> {
    insert_customer_audit_event(
        tx,
        NewCustomerAuditEvent {
            organization_id: org,
            project_id: project,
            actor_type: "human_user",
            actor_id: Some(auth.user_id.clone()),
            event_type: event,
            entity_type,
            entity_id,
            summary,
            metadata_json: "{}",
        },
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn role_matrix_is_fail_closed_and_explicit() {
        use OrganizationPermission as P;
        let all = [
            P::OrganizationRead,
            P::OrganizationManage,
            P::MembersRead,
            P::MembersManage,
            P::ProjectsRead,
            P::ProjectsManage,
            P::ApiKeysRead,
            P::ApiKeysManage,
            P::BillingRead,
            P::BillingManage,
            P::WorkloadsRead,
            P::WorkloadsWrite,
            P::ReservationsRead,
            P::ReservationsManage,
            P::ArtifactsRead,
            P::ArtifactsWrite,
            P::UsageRead,
            P::AuditRead,
        ];
        let cases: &[(OrganizationRole, &[P])] = &[
            (OrganizationRole::Owner, &all),
            (
                OrganizationRole::Admin,
                &[
                    P::OrganizationRead,
                    P::MembersRead,
                    P::MembersManage,
                    P::ProjectsRead,
                    P::ProjectsManage,
                    P::ApiKeysRead,
                    P::ApiKeysManage,
                    P::BillingRead,
                    P::WorkloadsRead,
                    P::WorkloadsWrite,
                    P::ReservationsRead,
                    P::ReservationsManage,
                    P::ArtifactsRead,
                    P::ArtifactsWrite,
                    P::UsageRead,
                    P::AuditRead,
                ],
            ),
            (
                OrganizationRole::BillingAdmin,
                &[
                    P::OrganizationRead,
                    P::BillingRead,
                    P::BillingManage,
                    P::UsageRead,
                ],
            ),
            (
                OrganizationRole::Developer,
                &[
                    P::OrganizationRead,
                    P::ProjectsRead,
                    P::ApiKeysRead,
                    P::ApiKeysManage,
                    P::WorkloadsRead,
                    P::WorkloadsWrite,
                    P::ReservationsRead,
                    P::ReservationsManage,
                    P::ArtifactsRead,
                    P::ArtifactsWrite,
                    P::UsageRead,
                ],
            ),
            (
                OrganizationRole::Viewer,
                &[
                    P::OrganizationRead,
                    P::ProjectsRead,
                    P::ApiKeysRead,
                    P::BillingRead,
                    P::WorkloadsRead,
                    P::ReservationsRead,
                    P::ArtifactsRead,
                    P::UsageRead,
                    P::AuditRead,
                ],
            ),
        ];
        for (role, expected) in cases {
            for permission in all {
                assert_eq!(
                    role.allows(permission),
                    expected.contains(&permission),
                    "unexpected {role:?} permission {permission:?}"
                );
            }
        }
        assert!(OrganizationRole::parse("future_role").is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_human_management_rbac_and_last_owner_race_fail_closed() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required");
        let schema = format!("burd_org_rbac_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        for user in [
            "user_owner_a",
            "user_owner_b",
            "user_developer",
            "user_viewer",
        ] {
            client.execute("INSERT INTO users(user_id,email,status,created_at,updated_at) VALUES($1,NULL,'active',$2,$2)",&[&user,&now]).await.unwrap();
        }
        drop(client);
        let owner_a = HumanSessionAuth {
            session_id: "session_a".into(),
            user_id: "user_owner_a".into(),
        };
        let owner_b = HumanSessionAuth {
            session_id: "session_b".into(),
            user_id: "user_owner_b".into(),
        };
        let developer = HumanSessionAuth {
            session_id: "session_d".into(),
            user_id: "user_developer".into(),
        };
        let viewer = HumanSessionAuth {
            session_id: "session_v".into(),
            user_id: "user_viewer".into(),
        };
        let org = db
            .create_human_organization(
                "req_org",
                &owner_a,
                &CreateOrganizationRequest {
                    display_name: "Human Org".into(),
                    owner_user_id: None,
                },
            )
            .await
            .unwrap()
            .organization;
        db.add_organization_member(
            "req_owner_b",
            &owner_a,
            &org.organization_id,
            &AddOrganizationMemberRequest {
                user_id: owner_b.user_id.clone(),
                role: "owner".into(),
            },
        )
        .await
        .unwrap();
        db.add_organization_member(
            "req_dev",
            &owner_a,
            &org.organization_id,
            &AddOrganizationMemberRequest {
                user_id: developer.user_id.clone(),
                role: "developer".into(),
            },
        )
        .await
        .unwrap();
        db.add_organization_member(
            "req_viewer",
            &owner_a,
            &org.organization_id,
            &AddOrganizationMemberRequest {
                user_id: viewer.user_id.clone(),
                role: "viewer".into(),
            },
        )
        .await
        .unwrap();
        let project = db
            .create_human_project(
                "req_project",
                &owner_a,
                &org.organization_id,
                &CreateProjectRequest {
                    display_name: "Project".into(),
                },
            )
            .await
            .unwrap()
            .project;
        let created = db
            .create_human_customer_api_key(
                "req_key",
                &developer,
                &project.project_id,
                &CreateCustomerApiKeyRequest::default(),
            )
            .await
            .unwrap();
        let listed = db
            .list_human_customer_api_keys("req_list", &viewer, &project.project_id)
            .await
            .unwrap();
        assert_eq!(listed.api_keys.len(), 1);
        let json = serde_json::to_value(&listed).unwrap().to_string();
        assert!(!json.contains(&created.token));
        assert!(!json.contains("key_hash"));
        assert!(
            db.create_human_project(
                "req_forbidden",
                &viewer,
                &org.organization_id,
                &CreateProjectRequest {
                    display_name: "No".into()
                }
            )
            .await
            .is_err()
        );
        let other_org = db
            .create_human_organization(
                "req_other_org",
                &owner_b,
                &CreateOrganizationRequest {
                    display_name: "Other Org".into(),
                    owner_user_id: None,
                },
            )
            .await
            .unwrap()
            .organization;
        assert!(
            db.list_organization_members("req_cross_org", &viewer, &other_org.organization_id)
                .await
                .is_err()
        );
        db.update_organization_member(
            "req_dev_inactive",
            &owner_a,
            &org.organization_id,
            &developer.user_id,
            &UpdateOrganizationMemberRequest {
                role: None,
                status: Some("inactive".into()),
            },
        )
        .await
        .unwrap();
        assert!(
            db.create_human_customer_api_key(
                "req_inactive",
                &developer,
                &project.project_id,
                &CreateCustomerApiKeyRequest::default()
            )
            .await
            .is_err()
        );
        db.update_organization_member(
            "req_dev_active",
            &owner_a,
            &org.organization_id,
            &developer.user_id,
            &UpdateOrganizationMemberRequest {
                role: None,
                status: Some("active".into()),
            },
        )
        .await
        .unwrap();
        db.revoke_human_customer_api_key(
            "req_revoke",
            &developer,
            &project.project_id,
            &created.api_key.api_key_id,
        )
        .await
        .unwrap();
        db.revoke_human_customer_api_key(
            "req_revoke_again",
            &developer,
            &project.project_id,
            &created.api_key.api_key_id,
        )
        .await
        .unwrap();
        assert!(
            db.authorize_customer_api_key(&created.token, Some(&project.project_id))
                .await
                .is_err()
        );

        // Both operations lock the developer's current membership row. The mutation may
        // serialize before the inactivation, but it may never authorize from stale state after
        // the inactivation transaction commits.
        let revoke_db = db.clone();
        let mutate_db = db.clone();
        let revoke_org = org.organization_id.clone();
        let mutate_project = project.project_id.clone();
        let revoke_owner = owner_a.clone();
        let mutate_developer = developer.clone();
        let revoke_target_user_id = mutate_developer.user_id.clone();
        let membership_revoke = tokio::spawn(async move {
            revoke_db
                .update_organization_member(
                    "req_membership_revoke_race",
                    &revoke_owner,
                    &revoke_org,
                    &revoke_target_user_id,
                    &UpdateOrganizationMemberRequest {
                        role: None,
                        status: Some("inactive".into()),
                    },
                )
                .await
        });
        let mutation = tokio::spawn(async move {
            mutate_db
                .create_human_customer_api_key(
                    "req_membership_mutation_race",
                    &developer,
                    &mutate_project,
                    &CreateCustomerApiKeyRequest::default(),
                )
                .await
        });
        let membership_revoke = membership_revoke.await.unwrap();
        assert!(membership_revoke.is_ok(), "{membership_revoke:?}");
        let mutation_serialized_before_or_denied = mutation.await.unwrap();
        assert!(matches!(
            mutation_serialized_before_or_denied,
            Ok(_) | Err(SessionError::Unauthorized)
        ));
        assert!(
            db.create_human_customer_api_key(
                "req_after_membership_revoke",
                &mutate_developer,
                &project.project_id,
                &CreateCustomerApiKeyRequest::default(),
            )
            .await
            .is_err()
        );

        db.update_organization_member(
            "req_restore_developer_before_role_race",
            &owner_a,
            &org.organization_id,
            &mutate_developer.user_id,
            &UpdateOrganizationMemberRequest {
                role: Some("developer".into()),
                status: Some("active".into()),
            },
        )
        .await
        .unwrap();
        // The same membership-row lock serializes role downgrade against authorization.
        let downgrade_db = db.clone();
        let role_mutate_db = db.clone();
        let downgrade_org = org.organization_id.clone();
        let role_project = project.project_id.clone();
        let downgrade_owner = owner_a.clone();
        let role_developer = mutate_developer.clone();
        let downgrade_target_user_id = role_developer.user_id.clone();
        let role_downgrade = tokio::spawn(async move {
            downgrade_db
                .update_organization_member(
                    "req_role_downgrade_race",
                    &downgrade_owner,
                    &downgrade_org,
                    &downgrade_target_user_id,
                    &UpdateOrganizationMemberRequest {
                        role: Some("viewer".into()),
                        status: None,
                    },
                )
                .await
        });
        let role_mutation = tokio::spawn(async move {
            role_mutate_db
                .create_human_customer_api_key(
                    "req_role_mutation_race",
                    &mutate_developer,
                    &role_project,
                    &CreateCustomerApiKeyRequest::default(),
                )
                .await
        });
        assert!(role_downgrade.await.unwrap().is_ok());
        let mutation_serialized_before_or_denied = role_mutation.await.unwrap();
        assert!(matches!(
            mutation_serialized_before_or_denied,
            Ok(_) | Err(SessionError::Unauthorized)
        ));
        assert!(
            db.create_human_customer_api_key(
                "req_after_role_downgrade",
                &role_developer,
                &project.project_id,
                &CreateCustomerApiKeyRequest::default(),
            )
            .await
            .is_err()
        );

        let db_a = db.clone();
        let db_b = db.clone();
        let org_a = org.organization_id.clone();
        let org_b = org.organization_id.clone();
        let task_a = tokio::spawn(async move {
            db_a.remove_organization_member("req_remove_a", &owner_b, &org_a, "user_owner_a")
                .await
        });
        let task_b = tokio::spawn(async move {
            db_b.remove_organization_member("req_remove_b", &owner_a, &org_b, "user_owner_b")
                .await
        });
        let results = [task_a.await.unwrap(), task_b.await.unwrap()];
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        let client = db.connect().await.unwrap();
        let owners:i64=client.query_one("SELECT COUNT(*) AS count FROM organization_users WHERE organization_id=$1 AND role='owner' AND status='active'", &[&org.organization_id]).await.unwrap().get("count");
        assert_eq!(owners, 1);
        let human_audits:i64=client.query_one("SELECT COUNT(*) AS count FROM customer_audit_events WHERE organization_id=$1 AND actor_type='human_user'", &[&org.organization_id]).await.unwrap().get("count");
        assert!(human_audits >= 6);
        drop(client);
        db.drop_schema_for_test().await.unwrap();
    }
}
