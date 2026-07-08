use crate::migrations::MIGRATIONS;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;
use tokio_postgres::{Client, NoTls, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Database {
    database_url: String,
    schema: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub provider_id: String,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyRecord {
    pub request_hash: String,
    pub status_code: u16,
    pub response_json: String,
}

#[derive(Debug)]
pub struct DbError {
    message: String,
}

impl DbError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DbError {}

impl From<tokio_postgres::Error> for DbError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl Database {
    pub fn new(database_url: String, schema: Option<String>) -> Result<Self, DbError> {
        if let Some(schema) = schema.as_deref() {
            validate_identifier(schema)?;
        }
        Ok(Self {
            database_url,
            schema,
        })
    }

    pub async fn migrate(&self) -> Result<(), DbError> {
        let client = self.connect().await?;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS schema_migrations (version TEXT PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL);",
            )
            .await?;
        for migration in MIGRATIONS {
            let exists = client
                .query_opt(
                    "SELECT version FROM schema_migrations WHERE version = $1",
                    &[&migration.version],
                )
                .await?
                .is_some();
            if exists {
                continue;
            }
            client.batch_execute(migration.sql).await?;
            client
                .execute(
                    "INSERT INTO schema_migrations (version, name, applied_at) VALUES ($1, $2, $3)",
                    &[
                        &migration.version,
                        &migration.name,
                        &Utc::now().to_rfc3339(),
                    ],
                )
                .await?;
        }
        Ok(())
    }

    pub async fn health_check(&self) -> Result<(), DbError> {
        let client = self.connect().await?;
        client.query_one("SELECT 1", &[]).await?;
        Ok(())
    }

    pub async fn migration_versions(&self) -> Result<Vec<String>, DbError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT version FROM schema_migrations ORDER BY version",
                &[],
            )
            .await?;
        Ok(rows.into_iter().map(|row| row.get("version")).collect())
    }

    pub async fn create_provider(
        &self,
        user_id: Option<String>,
        display_name: Option<String>,
    ) -> Result<ProviderRecord, DbError> {
        let client = self.connect().await?;
        let now = Utc::now().to_rfc3339();
        let provider = ProviderRecord {
            provider_id: format!("provider_{}", Uuid::new_v4()),
            user_id,
            display_name,
            status: "enrolled".to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        client
            .execute(
                "INSERT INTO providers (provider_id, user_id, display_name, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    &provider.provider_id,
                    &provider.user_id,
                    &provider.display_name,
                    &provider.status,
                    &provider.created_at,
                    &provider.updated_at,
                ],
            )
            .await?;
        Ok(provider)
    }

    pub async fn get_provider(&self, provider_id: &str) -> Result<Option<ProviderRecord>, DbError> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT provider_id, user_id, display_name, status, created_at, updated_at FROM providers WHERE provider_id = $1",
                &[&provider_id],
            )
            .await?;
        Ok(row.map(provider_from_row))
    }

    pub async fn insert_audit_event(&self, event: NewAuditEvent<'_>) -> Result<String, DbError> {
        let client = self.connect().await?;
        let audit_event_id = format!("audit_{}", Uuid::new_v4());
        client
            .execute(
                "INSERT INTO audit_events (audit_event_id, request_id, actor_type, actor_id, entity_type, entity_id, event_type, occurred_at, idempotency_key, summary, metadata_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                &[
                    &audit_event_id,
                    &event.request_id,
                    &event.actor_type,
                    &event.actor_id,
                    &event.entity_type,
                    &event.entity_id,
                    &event.event_type,
                    &Utc::now().to_rfc3339(),
                    &event.idempotency_key,
                    &event.summary,
                    &event.metadata_json,
                ],
            )
            .await?;
        Ok(audit_event_id)
    }

    pub async fn get_idempotency_record(
        &self,
        scope: &str,
        key: &str,
    ) -> Result<Option<IdempotencyRecord>, DbError> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT request_hash, status_code, response_json FROM idempotency_keys WHERE scope = $1 AND idempotency_key = $2",
                &[&scope, &key],
            )
            .await?;
        Ok(row.map(|row| IdempotencyRecord {
            request_hash: row.get("request_hash"),
            status_code: row.get::<_, i32>("status_code") as u16,
            response_json: row.get("response_json"),
        }))
    }

    pub async fn put_idempotency_record(
        &self,
        scope: &str,
        key: &str,
        request_hash: &str,
        status_code: u16,
        response_json: &str,
    ) -> Result<(), DbError> {
        let client = self.connect().await?;
        client
            .execute(
                "INSERT INTO idempotency_keys (scope, idempotency_key, request_hash, status_code, response_json, created_at) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (scope, idempotency_key) DO NOTHING",
                &[
                    &scope,
                    &key,
                    &request_hash,
                    &(status_code as i32),
                    &response_json,
                    &Utc::now().to_rfc3339(),
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn drop_schema_for_test(&self) -> Result<(), DbError> {
        let Some(schema) = self.schema.as_deref() else {
            return Ok(());
        };
        validate_identifier(schema)?;
        let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls).await?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .await?;
        Ok(())
    }

    async fn connect(&self) -> Result<Client, DbError> {
        let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "postgres_connection_error",
                        "error": error.to_string(),
                    })
                );
            }
        });
        if let Some(schema) = self.schema.as_deref() {
            validate_identifier(schema)?;
            client
                .batch_execute(&format!(
                    "CREATE SCHEMA IF NOT EXISTS {schema}; SET search_path TO {schema};"
                ))
                .await?;
        }
        Ok(client)
    }
}

pub struct NewAuditEvent<'a> {
    pub request_id: &'a str,
    pub actor_type: &'a str,
    pub actor_id: Option<String>,
    pub entity_type: &'a str,
    pub entity_id: &'a str,
    pub event_type: &'a str,
    pub idempotency_key: Option<String>,
    pub summary: &'a str,
    pub metadata_json: &'a str,
}

fn provider_from_row(row: Row) -> ProviderRecord {
    ProviderRecord {
        provider_id: row.get("provider_id"),
        user_id: row.get("user_id"),
        display_name: row.get("display_name"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn validate_identifier(identifier: &str) -> Result<(), DbError> {
    let valid = !identifier.is_empty()
        && identifier
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if valid {
        Ok(())
    } else {
        Err(DbError::new("database schema must be an ASCII identifier"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_schema_rejects_unsafe_identifiers() {
        assert!(
            Database::new(
                "postgres://localhost/db".to_string(),
                Some("ok_1".to_string())
            )
            .is_ok()
        );
        assert!(
            Database::new(
                "postgres://localhost/db".to_string(),
                Some("bad-name".to_string())
            )
            .is_err()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn migrates_and_persists_provider_with_isolated_schema() {
        let Ok(url) = std::env::var("BURD_CONTROL_TEST_DATABASE_URL") else {
            eprintln!("set BURD_CONTROL_TEST_DATABASE_URL to run this integration test");
            return;
        };
        let schema = format!("burd_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let provider = db
            .create_provider(None, Some("Integration Provider".to_string()))
            .await
            .unwrap();
        let loaded = db
            .get_provider(&provider.provider_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.provider_id, provider.provider_id);
        assert_eq!(loaded.status, "enrolled");

        db.drop_schema_for_test().await.unwrap();
    }
}
