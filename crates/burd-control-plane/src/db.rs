use crate::migrations::MIGRATIONS;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;
use tokio_postgres::{Client, NoTls, Row, Transaction};
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

#[derive(Debug, Clone)]
pub struct CreateProviderCommand {
    pub request_id: String,
    pub scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateProviderOutcome {
    Response(IdempotencyRecord),
    Conflict,
}

#[derive(Debug, Serialize)]
struct ProviderEnvelope {
    request_id: String,
    audit_event_id: Option<String>,
    provider: ProviderRecord,
}

#[derive(Debug)]
pub struct DbError {
    message: String,
}

impl DbError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
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
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .query_one(
                "SELECT pg_advisory_xact_lock(hashtext('burd_control_plane_migrations'))",
                &[],
            )
            .await?;
        transaction
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS schema_migrations (version TEXT PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL);",
            )
            .await?;
        for migration in MIGRATIONS {
            let exists = transaction
                .query_opt(
                    "SELECT version FROM schema_migrations WHERE version = $1",
                    &[&migration.version],
                )
                .await?
                .is_some();
            if exists {
                continue;
            }
            transaction.batch_execute(migration.sql).await?;
            transaction
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
        transaction.commit().await?;
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

    pub async fn create_provider_idempotently(
        &self,
        command: CreateProviderCommand,
    ) -> Result<CreateProviderOutcome, DbError> {
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let now = Utc::now().to_rfc3339();
        let reserved = transaction
            .execute(
                "INSERT INTO idempotency_keys (scope, idempotency_key, request_hash, status_code, response_json, created_at) VALUES ($1, $2, $3, 0, '', $4) ON CONFLICT (scope, idempotency_key) DO NOTHING",
                &[
                    &command.scope,
                    &command.idempotency_key,
                    &command.request_hash,
                    &now,
                ],
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
                Ok(CreateProviderOutcome::Response(record))
            } else {
                Ok(CreateProviderOutcome::Conflict)
            };
        }

        let provider = ProviderRecord {
            provider_id: format!("provider_{}", Uuid::new_v4()),
            user_id: command.user_id,
            display_name: command.display_name,
            status: "unregistered".to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        transaction
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

        let audit_event_id = insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id: &command.request_id,
                actor_type: "system",
                actor_id: None,
                entity_type: "provider",
                entity_id: &provider.provider_id,
                event_type: "provider.created",
                idempotency_key: Some(command.idempotency_key.clone()),
                summary: "provider registry record created",
                metadata_json: "{}",
            },
        )
        .await?;
        let response_json = serde_json::to_string(&ProviderEnvelope {
            request_id: command.request_id,
            audit_event_id: Some(audit_event_id),
            provider,
        })
        .map_err(|error| DbError::new(error.to_string()))?;
        let status_code = 201_i32;
        transaction
            .execute(
                "UPDATE idempotency_keys SET status_code = $1, response_json = $2 WHERE scope = $3 AND idempotency_key = $4",
                &[
                    &status_code,
                    &response_json,
                    &command.scope,
                    &command.idempotency_key,
                ],
            )
            .await?;
        transaction.commit().await?;

        Ok(CreateProviderOutcome::Response(IdempotencyRecord {
            request_hash: command.request_hash,
            status_code: status_code as u16,
            response_json,
        }))
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

    pub(crate) async fn connect(&self) -> Result<Client, DbError> {
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

pub(crate) async fn insert_audit_event(
    transaction: &Transaction<'_>,
    event: NewAuditEvent<'_>,
) -> Result<String, DbError> {
    let audit_event_id = format!("audit_{}", Uuid::new_v4());
    transaction
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

pub(crate) struct NewAuditEvent<'a> {
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

fn idempotency_from_row(row: Row) -> IdempotencyRecord {
    IdempotencyRecord {
        request_hash: row.get("request_hash"),
        status_code: row.get::<_, i32>("status_code") as u16,
        response_json: row.get("response_json"),
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
    async fn migrates_and_persists_provider_transactionally() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        assert_eq!(
            db.migration_versions().await.unwrap(),
            vec!["0001", "0002", "0003"]
        );

        let command = CreateProviderCommand {
            request_id: "req_integration".to_string(),
            scope: "POST /v1/providers".to_string(),
            idempotency_key: "provider-create-integration".to_string(),
            request_hash: "hash-one".to_string(),
            user_id: None,
            display_name: Some("Integration Provider".to_string()),
        };
        let first = db
            .create_provider_idempotently(command.clone())
            .await
            .unwrap();
        let CreateProviderOutcome::Response(first) = first else {
            panic!("first request must create the provider");
        };
        assert_eq!(first.status_code, 201);
        let response: serde_json::Value = serde_json::from_str(&first.response_json).unwrap();
        let provider_id = response["provider"]["provider_id"].as_str().unwrap();

        let loaded = db.get_provider(provider_id).await.unwrap().unwrap();
        assert_eq!(loaded.provider_id, provider_id);
        assert_eq!(loaded.status, "unregistered");

        let replayed = db
            .create_provider_idempotently(command.clone())
            .await
            .unwrap();
        let CreateProviderOutcome::Response(replayed) = replayed else {
            panic!("same request must replay the stored response");
        };
        assert_eq!(replayed.response_json, first.response_json);

        let mut conflicting = command;
        conflicting.request_hash = "hash-two".to_string();
        assert_eq!(
            db.create_provider_idempotently(conflicting).await.unwrap(),
            CreateProviderOutcome::Conflict
        );

        let client = db.connect().await.unwrap();
        let provider_count: i64 = client
            .query_one("SELECT COUNT(*) FROM providers", &[])
            .await
            .unwrap()
            .get(0);
        let audit_count: i64 = client
            .query_one("SELECT COUNT(*) FROM audit_events", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(provider_count, 1);
        assert_eq!(audit_count, 1);

        db.drop_schema_for_test().await.unwrap();
    }
}
