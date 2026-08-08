use crate::db::{Database, NewAuditEvent, insert_audit_event};
use crate::remote_session::{AuthorizedSession, SessionError};
use burd_protocol::{
    DEVICE_GPU_INVENTORY_CANONICALIZATION_VERSION, DEVICE_GPU_INVENTORY_SCHEMA_VERSION,
    DeviceGpuInventoryRecord, DeviceGpuInventoryVerification,
    ListProviderDeviceGpuInventoryResponse, SignedDeviceGpuInventory,
    SubmitDeviceGpuInventoryResponse, device_gpu_inventory_hash,
    device_gpu_inventory_signature_message, validate_device_gpu_inventory_payload, verify_message,
};
use chrono::Utc;
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

impl Database {
    pub async fn submit_device_gpu_inventory(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        signed: &SignedDeviceGpuInventory,
    ) -> Result<SubmitDeviceGpuInventoryResponse, SessionError> {
        validate_signed_inventory_shape(signed, authorized)?;
        let computed_hash =
            device_gpu_inventory_hash(&signed.payload).map_err(SessionError::Invalid)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let context = load_gpu_inventory_context(&transaction, authorized, signed).await?;
        let verification = verify_device_gpu_inventory(signed, &computed_hash, &context);
        if !verification.inventory_hash_valid
            || !verification.active_key_bound
            || !verification.signature_valid
            || !verification.session_bound
            || !verification.fingerprint_bound
        {
            record_rejected_device_gpu_inventory(
                &transaction,
                request_id,
                authorized,
                &computed_hash,
                signed,
                &verification,
            )
            .await?;
            transaction.commit().await?;
            if !verification.active_key_bound || !verification.session_bound {
                return Err(SessionError::Unauthorized);
            }
            if !verification.signature_valid {
                return Err(SessionError::SignatureInvalid);
            }
            return Err(SessionError::Invalid(verification.errors.join("; ")));
        }

        if let Some(records) = fetch_inventory_rows_by_hash(&transaction, &computed_hash).await? {
            transaction.commit().await?;
            return Ok(SubmitDeviceGpuInventoryResponse {
                request_id: request_id.to_string(),
                duplicate: true,
                records,
            });
        }

        let row_prefix = format!("device_gpu_inventory_{}", Uuid::new_v4());
        let payload_json = serde_json::to_string(&signed.payload)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let verification_json = serde_json::to_string(&verification)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let server_received_at = Utc::now().to_rfc3339();
        let mut inserted_rows = 0_u64;
        for (index, gpu) in signed.payload.gpus.iter().enumerate() {
            let inventory_row_id = format!("{row_prefix}_{index}");
            inserted_rows += transaction
                .execute(
                    "INSERT INTO device_gpu_inventory (inventory_row_id, provider_id, device_id, session_id, schema_version, inventory_hash, public_key_id, signature, canonicalization_version, gpu_uuid, gpu_index, backend, pci_vendor_id, pci_device_id, vram_total_mib, status, observed_at, server_received_at, payload_json, verification_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20) ON CONFLICT (inventory_hash, gpu_index) DO NOTHING",
                    &[
                        &inventory_row_id,
                        &authorized.provider_id,
                        &authorized.device_id,
                        &authorized.session_id,
                        &signed.payload.schema_version,
                        &computed_hash,
                        &signed.public_key_id,
                        &signed.signature,
                        &signed.canonicalization_version,
                        &gpu.gpu_uuid,
                        &(gpu.gpu_index as i32),
                        &gpu.backend,
                        &gpu.pci_vendor_id,
                        &gpu.pci_device_id,
                        &gpu.vram_total_mib.map(|value| value as i64),
                        &gpu.status,
                        &signed.payload.observed_at,
                        &server_received_at,
                        &payload_json,
                        &verification_json,
                    ],
                )
                .await?;
        }
        let duplicate = inserted_rows == 0;
        let audit_metadata = serde_json::json!({
            "inventory_hash": computed_hash,
            "gpu_count": signed.payload.gpus.len(),
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_key",
                actor_id: Some(signed.public_key_id.clone()),
                entity_type: "device_gpu_inventory",
                entity_id: &row_prefix,
                event_type: "device_gpu_inventory.accepted",
                idempotency_key: None,
                summary: "device GPU inventory accepted",
                metadata_json: &audit_metadata,
            },
        )
        .await?;
        let records = fetch_inventory_rows_by_hash(&transaction, &computed_hash)
            .await?
            .unwrap_or_default();
        transaction.commit().await?;

        Ok(SubmitDeviceGpuInventoryResponse {
            request_id: request_id.to_string(),
            duplicate,
            records,
        })
    }

    pub async fn list_provider_device_gpu_inventory(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListProviderDeviceGpuInventoryResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let limit = limit.clamp(1, 200) as i64;
        let client = self.connect().await?;
        if client
            .query_opt(
                "SELECT provider_id FROM providers WHERE provider_id = $1",
                &[&provider_id],
            )
            .await?
            .is_none()
        {
            return Err(SessionError::NotFound("provider not found".to_string()));
        }
        let rows = client
            .query(
                &format!(
                    "{} WHERE provider_id = $1 ORDER BY server_received_at DESC, gpu_index ASC LIMIT $2",
                    device_gpu_inventory_select_columns()
                ),
                &[&provider_id, &limit],
            )
            .await?;
        let records = rows
            .into_iter()
            .map(device_gpu_inventory_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListProviderDeviceGpuInventoryResponse {
            request_id: request_id.to_string(),
            provider_id: provider_id.to_string(),
            records,
        })
    }
}

#[derive(Debug)]
struct DeviceGpuInventoryContext {
    session_status: Option<String>,
    session_fingerprint: Option<String>,
    active_public_key: Option<String>,
}

async fn load_gpu_inventory_context(
    transaction: &Transaction<'_>,
    authorized: &AuthorizedSession,
    signed: &SignedDeviceGpuInventory,
) -> Result<DeviceGpuInventoryContext, SessionError> {
    let session = transaction
        .query_opt(
            "SELECT status, hardware_fingerprint FROM provider_sessions WHERE session_id = $1 AND provider_id = $2 AND device_id = $3 FOR UPDATE",
            &[&authorized.session_id, &authorized.provider_id, &authorized.device_id],
        )
        .await?;
    let active_public_key = transaction
        .query_opt(
            "SELECT public_key FROM provider_public_keys WHERE public_key_id = $1 AND provider_id = $2 AND device_id = $3 AND status = 'active'",
            &[&signed.public_key_id, &authorized.provider_id, &authorized.device_id],
        )
        .await?
        .map(|row| row.get::<_, String>("public_key"));
    Ok(DeviceGpuInventoryContext {
        session_status: session.as_ref().map(|row| row.get("status")),
        session_fingerprint: session.and_then(|row| row.get("hardware_fingerprint")),
        active_public_key,
    })
}

fn verify_device_gpu_inventory(
    signed: &SignedDeviceGpuInventory,
    computed_hash: &str,
    context: &DeviceGpuInventoryContext,
) -> DeviceGpuInventoryVerification {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let inventory_hash_valid = computed_hash == signed.inventory_hash;
    if !inventory_hash_valid {
        errors.push("inventory_hash does not match canonical payload".to_string());
    }
    let session_bound = context
        .session_status
        .as_deref()
        .is_some_and(|status| matches!(status, "online" | "degraded"));
    if !session_bound {
        errors.push("GPU inventory requires an online or degraded remote session".to_string());
    }
    let fingerprint_bound = context.session_fingerprint.as_deref()
        == Some(signed.payload.hardware_fingerprint.as_str());
    if !fingerprint_bound {
        errors.push(
            "GPU inventory hardware fingerprint does not match the remote session".to_string(),
        );
    }
    let active_key_bound = context.active_public_key.is_some();
    if !active_key_bound {
        errors.push("GPU inventory public_key_id is not active for this device".to_string());
    }
    let signature_message = device_gpu_inventory_signature_message(
        &signed.payload,
        computed_hash,
        &signed.public_key_id,
    )
    .unwrap_or_default();
    let signature_valid = context
        .active_public_key
        .as_ref()
        .is_some_and(|public_key| {
            verify_message(public_key, signature_message.as_bytes(), &signed.signature)
                .unwrap_or(false)
        });
    if !signature_valid {
        errors.push("GPU inventory signature is invalid".to_string());
    }
    let active_gpu_count = signed
        .payload
        .gpus
        .iter()
        .filter(|gpu| gpu.status == "active")
        .count();
    if active_gpu_count == 0 {
        warnings.push("inventory snapshot does not contain any active GPUs".to_string());
    }

    DeviceGpuInventoryVerification {
        schema_version: DEVICE_GPU_INVENTORY_SCHEMA_VERSION.to_string(),
        inventory_hash_valid,
        signature_valid,
        session_bound,
        fingerprint_bound,
        active_key_bound,
        warnings,
        errors,
    }
}

async fn record_rejected_device_gpu_inventory(
    transaction: &Transaction<'_>,
    request_id: &str,
    authorized: &AuthorizedSession,
    inventory_hash: &str,
    signed: &SignedDeviceGpuInventory,
    verification: &DeviceGpuInventoryVerification,
) -> Result<(), SessionError> {
    let metadata = serde_json::json!({
        "inventory_hash": inventory_hash,
        "public_key_id": signed.public_key_id,
        "errors": &verification.errors,
    })
    .to_string();
    insert_audit_event(
        transaction,
        NewAuditEvent {
            request_id,
            actor_type: "device_key",
            actor_id: Some(signed.public_key_id.clone()),
            entity_type: "device_gpu_inventory",
            entity_id: &authorized.device_id,
            event_type: "device_gpu_inventory.rejected",
            idempotency_key: None,
            summary: "device GPU inventory rejected",
            metadata_json: &metadata,
        },
    )
    .await?;
    Ok(())
}

fn validate_signed_inventory_shape(
    signed: &SignedDeviceGpuInventory,
    authorized: &AuthorizedSession,
) -> Result<(), SessionError> {
    let payload = &signed.payload;
    if signed.canonicalization_version != DEVICE_GPU_INVENTORY_CANONICALIZATION_VERSION {
        return Err(SessionError::Invalid(
            "unsupported device GPU inventory schema or canonicalization version".to_string(),
        ));
    }
    validate_device_gpu_inventory_payload(payload).map_err(SessionError::Invalid)?;
    if payload.provider_id != authorized.provider_id
        || payload.device_id != authorized.device_id
        || payload.session_id != authorized.session_id
    {
        return Err(SessionError::Unauthorized);
    }
    if signed.public_key_id.trim().is_empty()
        || signed.signature.trim().is_empty()
        || signed.inventory_hash.trim().is_empty()
    {
        return Err(SessionError::Invalid(
            "device GPU inventory signature fields are required".to_string(),
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
fn device_gpu_inventory_select_columns() -> &'static str {
    "SELECT inventory_row_id, provider_id, device_id, session_id, schema_version, inventory_hash, public_key_id, canonicalization_version, gpu_uuid, gpu_index, backend, pci_vendor_id, pci_device_id, vram_total_mib, status, observed_at, server_received_at, verification_json FROM device_gpu_inventory"
}

async fn fetch_inventory_rows_by_hash(
    transaction: &Transaction<'_>,
    inventory_hash: &str,
) -> Result<Option<Vec<DeviceGpuInventoryRecord>>, SessionError> {
    let rows = transaction
        .query(
            &format!(
                "{} WHERE inventory_hash = $1 ORDER BY gpu_index ASC",
                device_gpu_inventory_select_columns()
            ),
            &[&inventory_hash],
        )
        .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    rows.into_iter()
        .map(device_gpu_inventory_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn device_gpu_inventory_from_row(row: Row) -> Result<DeviceGpuInventoryRecord, SessionError> {
    let verification_json: String = row.get("verification_json");
    let verification = serde_json::from_str::<DeviceGpuInventoryVerification>(&verification_json)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    let gpu_index: i32 = row.get("gpu_index");
    let vram_total_mib: Option<i64> = row.get("vram_total_mib");
    Ok(DeviceGpuInventoryRecord {
        inventory_row_id: row.get("inventory_row_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        schema_version: row.get("schema_version"),
        inventory_hash: row.get("inventory_hash"),
        public_key_id: row.get("public_key_id"),
        canonicalization_version: row.get("canonicalization_version"),
        gpu_uuid: row.get("gpu_uuid"),
        gpu_index: gpu_index as u32,
        backend: row.get("backend"),
        pci_vendor_id: row.get("pci_vendor_id"),
        pci_device_id: row.get("pci_device_id"),
        vram_total_mib: vram_total_mib.map(|value| value as u64),
        status: row.get("status"),
        observed_at: row.get("observed_at"),
        server_received_at: row.get("server_received_at"),
        verification,
    })
}

pub(crate) async fn assert_gpu_inventory_contains(
    transaction: &Transaction<'_>,
    provider_id: &str,
    device_id: &str,
    gpu_uuid: &str,
) -> Result<(), SessionError> {
    let latest_status = transaction
        .query_opt(
            "SELECT status FROM device_gpu_inventory WHERE provider_id = $1 AND device_id = $2 AND lower(gpu_uuid) = lower($3) AND inventory_hash = (SELECT inventory_hash FROM device_gpu_inventory WHERE provider_id = $1 AND device_id = $2 ORDER BY server_received_at DESC, observed_at DESC LIMIT 1)",
            &[&provider_id, &device_id, &gpu_uuid],
        )
        .await?
        .map(|row| row.get::<_, String>("status"));
    if latest_status.as_deref() == Some("active") {
        Ok(())
    } else {
        Err(SessionError::Conflict(
            "requested GPU is not active in the latest device inventory".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{
        DEVICE_GPU_INVENTORY_CANONICALIZATION_VERSION, DEVICE_GPU_INVENTORY_SCHEMA_VERSION,
        DeviceGpuInventoryGpu, DeviceGpuInventoryPayload, SignedDeviceGpuInventory,
        device_gpu_inventory_hash,
    };
    use chrono::Duration;

    fn signed_inventory() -> SignedDeviceGpuInventory {
        let payload = DeviceGpuInventoryPayload {
            schema_version: DEVICE_GPU_INVENTORY_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            observed_at: "2026-07-14T00:00:00Z".to_string(),
            gpus: vec![
                DeviceGpuInventoryGpu {
                    gpu_uuid: "GPU-1".to_string(),
                    gpu_index: 0,
                    backend: "cuda".to_string(),
                    pci_vendor_id: "10de".to_string(),
                    pci_device_id: "2684".to_string(),
                    vram_total_mib: Some(24_576),
                    status: "active".to_string(),
                },
                DeviceGpuInventoryGpu {
                    gpu_uuid: "GPU-2".to_string(),
                    gpu_index: 1,
                    backend: "cuda".to_string(),
                    pci_vendor_id: "10de".to_string(),
                    pci_device_id: "2684".to_string(),
                    vram_total_mib: Some(24_576),
                    status: "active".to_string(),
                },
            ],
        };
        let inventory_hash = device_gpu_inventory_hash(&payload).unwrap();
        SignedDeviceGpuInventory {
            payload,
            inventory_hash,
            public_key_id: "key_1".to_string(),
            signature: "signature".to_string(),
            canonicalization_version: DEVICE_GPU_INVENTORY_CANONICALIZATION_VERSION.to_string(),
        }
    }

    #[test]
    fn validation_accepts_a_multi_gpu_snapshot() {
        let inventory = signed_inventory();
        let authorized = AuthorizedSession {
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            sequence_last: 0,
            heartbeat_interval_seconds: 30,
            missed_heartbeat_limit: 3,
        };
        assert!(validate_signed_inventory_shape(&inventory, &authorized).is_ok());
    }
    async fn setup_inventory_database() -> Database {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_gpu_inventory_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let now = Utc::now().to_rfc3339();
        let expires_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let client = db.connect().await.unwrap();
        client
            .execute(
                "INSERT INTO providers (provider_id, user_id, display_name, status, created_at, updated_at) VALUES ('provider_1', NULL, 'GPU Provider', 'available', $1, $1)",
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
                "INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at) VALUES ('key_1', 'provider_1', 'device_1', 'pub_1', 'ed25519', 'active', $1)",
                &[&now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint) VALUES ('session_1', 'provider_1', 'device_1', 'online', 0, $1, $2, 'sha256:fingerprint')",
                &[&now, &expires_at],
            )
            .await
            .unwrap();
        drop(client);
        db
    }

    async fn insert_inventory_row(
        client: &tokio_postgres::Client,
        inventory_hash: &str,
        inventory_row_id: &str,
        gpu_uuid: &str,
        gpu_index: i32,
        status: &str,
        observed_at: &str,
    ) {
        client
            .execute(
                "INSERT INTO device_gpu_inventory (inventory_row_id, provider_id, device_id, session_id, schema_version, inventory_hash, public_key_id, signature, canonicalization_version, gpu_uuid, gpu_index, backend, pci_vendor_id, pci_device_id, vram_total_mib, status, observed_at, server_received_at, payload_json, verification_json) VALUES ($1, 'provider_1', 'device_1', 'session_1', 'burd-device-gpu-inventory-v1', $2, 'key_1', 'signature_1', 'burd-json-c14n-v1', $3, $4, 'cuda', '10de', '2684', 24576, $5, $6, $7, '{}', '{}')",
                &[
                    &inventory_row_id,
                    &inventory_hash,
                    &gpu_uuid,
                    &gpu_index,
                    &status,
                    &observed_at,
                    &observed_at,
                ],
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_inventory_schema_allows_one_row_per_gpu_for_same_snapshot_hash() {
        let db = setup_inventory_database().await;
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        insert_inventory_row(
            &client,
            "inventory_hash_shared",
            "inventory_1",
            "GPU-1",
            0,
            "active",
            &now,
        )
        .await;
        insert_inventory_row(
            &client,
            "inventory_hash_shared",
            "inventory_2",
            "GPU-2",
            1,
            "active",
            &now,
        )
        .await;
        let count: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM device_gpu_inventory WHERE inventory_hash = 'inventory_hash_shared'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 2);
        drop(client);
        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_inventory_presence_uses_complete_latest_snapshot() {
        let db = setup_inventory_database().await;
        let mut client = db.connect().await.unwrap();
        let first = Utc::now().to_rfc3339();
        let second = (Utc::now() + Duration::seconds(1)).to_rfc3339();
        insert_inventory_row(
            &client,
            "inventory_hash_active",
            "inventory_active",
            "GPU-stale",
            0,
            "active",
            &first,
        )
        .await;
        insert_inventory_row(
            &client,
            "inventory_hash_current",
            "inventory_current",
            "GPU-current",
            0,
            "active",
            &second,
        )
        .await;

        let transaction = client.transaction().await.unwrap();
        let result =
            assert_gpu_inventory_contains(&transaction, "provider_1", "device_1", "GPU-stale")
                .await;
        assert!(matches!(result, Err(SessionError::Conflict(_))));
        assert!(
            assert_gpu_inventory_contains(&transaction, "provider_1", "device_1", "gpu-CURRENT",)
                .await
                .is_ok()
        );
        transaction.commit().await.unwrap();
        drop(client);
        db.drop_schema_for_test().await.unwrap();
    }
}
