use crate::db::{Database, DbError};
use crate::remote_session::SessionError;
use burd_protocol::{
    HUMAN_AUTH_SCHEMA_VERSION, HumanIdentitySummary, HumanMeResponse,
    HumanOrganizationMembershipSummary, random_token, sha256_hex,
};
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::Transaction;
use uuid::Uuid;

pub const GOOGLE_OIDC_PROVIDER: &str = "google";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanSessionAuth {
    pub session_id: String,
    pub user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedGoogleIdentity {
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CreatedHumanSession {
    pub auth: HumanSessionAuth,
    pub token: String,
}

impl Database {
    pub async fn create_google_human_session(
        &self,
        request_id: &str,
        identity: &VerifiedGoogleIdentity,
        ttl_seconds: u32,
    ) -> Result<CreatedHumanSession, SessionError> {
        validate_google_identity(identity)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        transaction
            .query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
                &[&format!("google:{}", identity.subject)],
            )
            .await?;
        let row = transaction
            .query_opt(
                "SELECT i.oidc_identity_id, i.user_id, u.status FROM human_oidc_identities i JOIN users u ON u.user_id = i.user_id WHERE i.provider = 'google' AND i.provider_subject = $1 FOR UPDATE OF i, u",
                &[&identity.subject],
            )
            .await?;
        let user_id = if let Some(row) = row {
            let status: String = row.get("status");
            if status != "active" {
                return Err(SessionError::Unauthorized);
            }
            let oidc_identity_id: String = row.get("oidc_identity_id");
            let user_id: String = row.get("user_id");
            transaction
                .execute(
                    "UPDATE human_oidc_identities SET email = $1, email_verified = $2, updated_at = $3, last_login_at = $3 WHERE oidc_identity_id = $4",
                    &[&identity.email, &identity.email_verified, &now_text, &oidc_identity_id],
                )
                .await?;
            user_id
        } else {
            let user_id = format!("user_{}", Uuid::new_v4());
            let identity_id = format!("oidc_{}", Uuid::new_v4());
            // Email remains identity metadata. It is deliberately not copied to users.email,
            // which prevents legacy email collisions from becoming account linking.
            transaction
                .execute(
                    "INSERT INTO users (user_id, email, status, created_at, updated_at) VALUES ($1, NULL, 'active', $2, $2)",
                    &[&user_id, &now_text],
                )
                .await?;
            transaction
                .execute(
                    "INSERT INTO human_oidc_identities (oidc_identity_id, user_id, provider, provider_subject, email, email_verified, created_at, updated_at, last_login_at) VALUES ($1, $2, 'google', $3, $4, $5, $6, $6, $6)",
                    &[&identity_id, &user_id, &identity.subject, &identity.email, &identity.email_verified, &now_text],
                )
                .await?;
            insert_human_audit(
                &transaction,
                request_id,
                &user_id,
                "human_oidc_identity.created",
                "human OIDC identity provisioned",
            )
            .await?;
            user_id
        };
        let created = insert_session(&transaction, &user_id, now, ttl_seconds).await?;
        insert_human_audit(
            &transaction,
            request_id,
            &user_id,
            "human_session.created",
            "human session created",
        )
        .await?;
        transaction.commit().await?;
        Ok(created)
    }

    pub async fn authorize_human_session(
        &self,
        token: &str,
    ) -> Result<HumanSessionAuth, SessionError> {
        if token.is_empty() || token.len() > 256 {
            return Err(SessionError::Unauthorized);
        }
        let token_hash = sha256_hex(token.as_bytes());
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_opt(
                "SELECT s.session_id, s.user_id, s.status, s.expires_at, u.status AS user_status FROM human_sessions s JOIN users u ON u.user_id = s.user_id WHERE s.token_hash = $1 FOR UPDATE OF s",
                &[&token_hash],
            )
            .await?
            .ok_or(SessionError::Unauthorized)?;
        let session_status: String = row.get("status");
        let user_status: String = row.get("user_status");
        let expires_at: String = row.get("expires_at");
        let expires_at = DateTime::parse_from_rfc3339(&expires_at)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?
            .with_timezone(&Utc);
        if session_status != "active" || user_status != "active" || expires_at <= Utc::now() {
            return Err(SessionError::Unauthorized);
        }
        let auth = HumanSessionAuth {
            session_id: row.get("session_id"),
            user_id: row.get("user_id"),
        };
        transaction
            .execute(
                "UPDATE human_sessions SET last_seen_at = $1 WHERE session_id = $2 AND (last_seen_at IS NULL OR last_seen_at < $3)",
                &[&Utc::now().to_rfc3339(), &auth.session_id, &(Utc::now() - Duration::minutes(5)).to_rfc3339()],
            )
            .await?;
        transaction.commit().await?;
        Ok(auth)
    }

    pub async fn human_me(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
    ) -> Result<HumanMeResponse, SessionError> {
        let client = self.connect().await?;
        let user = client
            .query_one(
                "SELECT status FROM users WHERE user_id = $1",
                &[&auth.user_id],
            )
            .await?;
        let identity = client
            .query_one(
                "SELECT provider, email, email_verified FROM human_oidc_identities WHERE user_id = $1 ORDER BY created_at ASC LIMIT 1",
                &[&auth.user_id],
            )
            .await?;
        let memberships = client
            .query(
                "SELECT organization_id, role, status FROM organization_users WHERE user_id = $1 ORDER BY organization_id ASC",
                &[&auth.user_id],
            )
            .await?
            .into_iter()
            .map(|row| HumanOrganizationMembershipSummary {
                organization_id: row.get("organization_id"),
                role: row.get("role"),
                status: row.get("status"),
            })
            .collect();
        Ok(HumanMeResponse {
            request_id: request_id.to_string(),
            schema_version: HUMAN_AUTH_SCHEMA_VERSION.to_string(),
            user_id: auth.user_id.clone(),
            status: user.get("status"),
            identity: HumanIdentitySummary {
                provider: identity.get("provider"),
                email: identity.get("email"),
                email_verified: identity.get("email_verified"),
            },
            organization_memberships: memberships,
        })
    }

    pub async fn revoke_human_session(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
    ) -> Result<u64, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        let count = transaction.execute("UPDATE human_sessions SET status = 'revoked', revoked_at = COALESCE(revoked_at, $1) WHERE session_id = $2 AND status = 'active'", &[&now, &auth.session_id]).await?;
        insert_human_audit(
            &transaction,
            request_id,
            &auth.user_id,
            "human_session.revoked",
            "human session revoked",
        )
        .await?;
        transaction.commit().await?;
        Ok(count)
    }

    pub async fn revoke_all_human_sessions(
        &self,
        request_id: &str,
        auth: &HumanSessionAuth,
    ) -> Result<u64, SessionError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .query_one(
                "SELECT user_id FROM users WHERE user_id = $1 FOR UPDATE",
                &[&auth.user_id],
            )
            .await?;
        let now = Utc::now().to_rfc3339();
        let count = transaction.execute("UPDATE human_sessions SET status = 'revoked', revoked_at = COALESCE(revoked_at, $1) WHERE user_id = $2 AND status = 'active'", &[&now, &auth.user_id]).await?;
        insert_human_audit(
            &transaction,
            request_id,
            &auth.user_id,
            "human_session.revoked_all",
            "all human sessions revoked",
        )
        .await?;
        transaction.commit().await?;
        Ok(count)
    }
}

async fn insert_session(
    transaction: &Transaction<'_>,
    user_id: &str,
    now: DateTime<Utc>,
    ttl_seconds: u32,
) -> Result<CreatedHumanSession, SessionError> {
    let token = random_token("burd_human").map_err(SessionError::Invalid)?;
    let token_hash = sha256_hex(token.as_bytes());
    let session_id = format!("hsess_{}", Uuid::new_v4());
    transaction.execute("INSERT INTO human_sessions (session_id, user_id, token_hash, status, created_at, expires_at) VALUES ($1, $2, $3, 'active', $4, $5)", &[&session_id, &user_id, &token_hash, &now.to_rfc3339(), &(now + Duration::seconds(i64::from(ttl_seconds))).to_rfc3339()]).await?;
    Ok(CreatedHumanSession {
        auth: HumanSessionAuth {
            session_id,
            user_id: user_id.to_string(),
        },
        token,
    })
}

fn validate_google_identity(identity: &VerifiedGoogleIdentity) -> Result<(), SessionError> {
    if identity.subject.is_empty() || identity.subject.len() > 255 || !identity.subject.is_ascii() {
        return Err(SessionError::Unauthorized);
    }
    if identity.email.is_none() || !identity.email_verified {
        return Err(SessionError::Unauthorized);
    }
    if identity
        .email
        .as_ref()
        .is_some_and(|email| email.len() > 254 || !email.is_ascii())
    {
        return Err(SessionError::Unauthorized);
    }
    Ok(())
}

async fn insert_human_audit(
    transaction: &Transaction<'_>,
    request_id: &str,
    user_id: &str,
    event_type: &str,
    summary: &str,
) -> Result<(), SessionError> {
    transaction.execute("INSERT INTO audit_events (audit_event_id, request_id, actor_type, actor_id, entity_type, entity_id, event_type, summary, metadata_json, occurred_at) VALUES ($1, $2, 'human_user', $3, 'user', $3, $4, $5, '{}', $6)", &[&format!("audit_{}", Uuid::new_v4()), &request_id, &user_id, &event_type, &summary, &Utc::now().to_rfc3339()]).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unverified_email_is_fail_closed() {
        let identity = VerifiedGoogleIdentity {
            subject: "123".to_string(),
            email: Some("a@example.com".to_string()),
            email_verified: false,
        };
        assert!(validate_google_identity(&identity).is_err());
        let missing = VerifiedGoogleIdentity {
            subject: "123".to_string(),
            email: None,
            email_verified: false,
        };
        assert!(validate_google_identity(&missing).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_google_jit_and_human_session_lifecycle_are_authoritative() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required");
        let schema = format!("burd_human_auth_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        client.execute("INSERT INTO users(user_id,email,status,created_at,updated_at) VALUES('user_legacy','same@example.test','active',$1,$1)",&[&now]).await.unwrap();
        drop(client);
        let identity = VerifiedGoogleIdentity {
            subject: "google-subject-1".into(),
            email: Some("same@example.test".into()),
            email_verified: true,
        };
        let first = db
            .create_google_human_session("req_jit_1", &identity, 3600)
            .await
            .unwrap();
        assert_ne!(first.auth.user_id, "user_legacy");
        let second = db
            .create_google_human_session("req_jit_2", &identity, 3600)
            .await
            .unwrap();
        assert_eq!(first.auth.user_id, second.auth.user_id);
        assert_ne!(first.auth.session_id, second.auth.session_id);
        let other = db
            .create_google_human_session(
                "req_jit_3",
                &VerifiedGoogleIdentity {
                    subject: "google-subject-2".into(),
                    email: Some("same@example.test".into()),
                    email_verified: true,
                },
                3600,
            )
            .await
            .unwrap();
        assert_ne!(other.auth.user_id, first.auth.user_id);
        assert_ne!(other.auth.user_id, "user_legacy");
        let client = db.connect().await.unwrap();
        let row = client
            .query_one(
                "SELECT token_hash FROM human_sessions WHERE session_id=$1",
                &[&first.auth.session_id],
            )
            .await
            .unwrap();
        let hash: String = row.get("token_hash");
        assert_eq!(hash, sha256_hex(first.token.as_bytes()));
        assert_ne!(hash, first.token);
        let columns=client.query("SELECT column_name FROM information_schema.columns WHERE table_schema=current_schema() AND table_name IN ('human_oidc_identities','human_sessions')",&[]).await.unwrap().into_iter().map(|r|r.get::<_,String>("column_name")).collect::<Vec<_>>();
        for forbidden in [
            "access_token",
            "refresh_token",
            "id_token",
            "authorization_code",
            "pkce_verifier",
        ] {
            assert!(!columns.iter().any(|column| column == forbidden));
        }
        drop(client);
        assert_eq!(
            db.authorize_human_session(&first.token).await.unwrap(),
            first.auth
        );
        let revoked = db
            .revoke_all_human_sessions("req_logout_all", &first.auth)
            .await
            .unwrap();
        assert_eq!(revoked, 2);
        assert!(db.authorize_human_session(&first.token).await.is_err());
        assert!(db.authorize_human_session(&second.token).await.is_err());
        let expired = db
            .create_google_human_session("req_jit_expired", &identity, 3600)
            .await
            .unwrap();
        let client = db.connect().await.unwrap();
        client
            .execute(
                "UPDATE human_sessions SET created_at=$1, expires_at=$2 WHERE session_id=$3",
                &[
                    &(Utc::now() - Duration::hours(2)).to_rfc3339(),
                    &(Utc::now() - Duration::hours(1)).to_rfc3339(),
                    &expired.auth.session_id,
                ],
            )
            .await
            .unwrap();
        drop(client);
        assert!(db.authorize_human_session(&expired.token).await.is_err());
        let active = db
            .create_google_human_session("req_jit_4", &identity, 3600)
            .await
            .unwrap();
        let client = db.connect().await.unwrap();
        client
            .execute(
                "UPDATE users SET status='disabled' WHERE user_id=$1",
                &[&active.auth.user_id],
            )
            .await
            .unwrap();
        drop(client);
        assert!(db.authorize_human_session(&active.token).await.is_err());
        db.drop_schema_for_test().await.unwrap();
    }
}
