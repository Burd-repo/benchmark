use crate::db::{Database, NewAuditEvent, insert_audit_event};
use crate::remote_session::{AuthorizedSession, SessionError};
use burd_protocol::{
    ClientControlMessage, GpuTelemetrySample, LatestTelemetryResponse, SignedTelemetryBatch,
    TELEMETRY_CANONICALIZATION_VERSION, TELEMETRY_SCHEMA_VERSION, TelemetryBatchReceipt,
    telemetry_batch_hash, telemetry_batch_signature_message, verify_message,
};
use chrono::{Duration, Utc};
use tokio_postgres::Transaction;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct TelemetryPolicy {
    pub max_samples_per_batch: u32,
    pub min_batch_interval_seconds: u32,
    pub clock_skew_seconds: u32,
}

impl Database {
    pub async fn ingest_gpu_telemetry(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        message: &ClientControlMessage,
        policy: TelemetryPolicy,
    ) -> Result<TelemetryBatchReceipt, SessionError> {
        if message.session_id != authorized.session_id || message.device_id != authorized.device_id
        {
            return Err(SessionError::Unauthorized);
        }
        if message.message_type != "telemetry_batch" {
            return Err(SessionError::Invalid(
                "control message is not a telemetry_batch".to_string(),
            ));
        }
        let signed: SignedTelemetryBatch = serde_json::from_value(message.payload.clone())
            .map_err(|error| {
                SessionError::Invalid(format!("invalid signed telemetry batch: {error}"))
            })?;
        validate_batch_contract(&signed, message, authorized, policy)?;

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let session = transaction
            .query_opt(
                "SELECT status, sequence_last, hardware_fingerprint FROM provider_sessions WHERE session_id = $1 FOR UPDATE",
                &[&authorized.session_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
        let status: String = session.get("status");
        if !matches!(status.as_str(), "online" | "degraded") {
            return Err(SessionError::Conflict(
                "telemetry requires an online or degraded remote session".to_string(),
            ));
        }
        let expected_fingerprint: Option<String> = session.get("hardware_fingerprint");
        if expected_fingerprint.as_deref() != Some(&signed.payload.hardware_fingerprint) {
            return Err(SessionError::Conflict(
                "telemetry hardware fingerprint does not match the remote session".to_string(),
            ));
        }
        let sequence_last = session.get::<_, i64>("sequence_last").max(0) as u64;
        if message.sequence <= sequence_last {
            return Err(SessionError::Conflict(format!(
                "control sequence {} was already observed; last sequence is {sequence_last}",
                message.sequence
            )));
        }
        let control_gap = message.sequence.saturating_sub(sequence_last + 1);
        let last_sample_sequence = transaction
            .query_opt(
                "SELECT sample_sequence_end FROM telemetry_batches WHERE session_id = $1 ORDER BY sample_sequence_end DESC LIMIT 1 FOR UPDATE",
                &[&authorized.session_id],
            )
            .await?
            .map(|row| row.get::<_, i64>("sample_sequence_end").max(0) as u64)
            .unwrap_or(0);
        if signed.payload.sample_sequence_start != last_sample_sequence + 1 {
            return Err(SessionError::Conflict(format!(
                "telemetry sample sequence must start at {}; received {}",
                last_sample_sequence + 1,
                signed.payload.sample_sequence_start
            )));
        }
        enforce_batch_interval(
            &transaction,
            &authorized.session_id,
            policy.min_batch_interval_seconds,
        )
        .await?;
        verify_batch_signature(&transaction, authorized, &signed).await?;

        let server_received_at = Utc::now().to_rfc3339();
        let batch_id = format!("telemetry_batch_{}", Uuid::new_v4());
        let payload_json = serde_json::to_string(&signed.payload)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let verification_json = serde_json::json!({
            "hash_valid": true,
            "signature_valid": true,
            "session_bound": true,
            "fingerprint_bound": true,
            "server_received_at": server_received_at,
        })
        .to_string();
        transaction
            .execute(
                "INSERT INTO telemetry_batches (batch_id, provider_id, device_id, session_id, control_sequence, sample_sequence_start, sample_sequence_end, hardware_fingerprint, collector, sample_count, batch_hash, public_key_id, signature, canonicalization_version, collected_at_start, collected_at_end, server_received_at, payload_json, verification_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)",
                &[&batch_id, &authorized.provider_id, &authorized.device_id, &authorized.session_id, &(message.sequence as i64), &(signed.payload.sample_sequence_start as i64), &(signed.payload.sample_sequence_end as i64), &signed.payload.hardware_fingerprint, &signed.payload.collector, &(signed.payload.samples.len() as i32), &signed.batch_hash, &signed.public_key_id, &signed.signature, &signed.canonicalization_version, &signed.payload.collected_at_start, &signed.payload.collected_at_end, &server_received_at, &payload_json, &verification_json],
            )
            .await?;
        for sample in &signed.payload.samples {
            insert_sample(
                &transaction,
                &batch_id,
                authorized,
                &server_received_at,
                sample,
            )
            .await?;
        }
        let next_status = if control_gap > 0 {
            "degraded"
        } else {
            status.as_str()
        };
        transaction
            .execute(
                "UPDATE provider_sessions SET status = $1, sequence_last = $2, degraded_at = CASE WHEN $1 = 'degraded' THEN $3 ELSE degraded_at END, updated_at = $3 WHERE session_id = $4",
                &[&next_status, &(message.sequence as i64), &server_received_at, &authorized.session_id],
            )
            .await?;
        let audit_metadata = serde_json::json!({
            "batch_id": batch_id,
            "sample_count": signed.payload.samples.len(),
            "control_gap": control_gap,
            "collector": signed.payload.collector,
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_key",
                actor_id: Some(signed.public_key_id.clone()),
                entity_type: "telemetry_batch",
                entity_id: &batch_id,
                event_type: "telemetry_batch.accepted",
                idempotency_key: None,
                summary: "signed GPU telemetry batch accepted",
                metadata_json: &audit_metadata,
            },
        )
        .await?;
        transaction.commit().await?;

        Ok(TelemetryBatchReceipt {
            request_id: request_id.to_string(),
            batch_id,
            session_id: authorized.session_id.clone(),
            control_sequence_ack: message.sequence,
            sample_sequence_end: signed.payload.sample_sequence_end,
            sample_count: signed.payload.samples.len(),
            batch_hash: signed.batch_hash,
            status: "accepted".to_string(),
            server_received_at,
        })
    }

    pub async fn latest_gpu_telemetry(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
    ) -> Result<LatestTelemetryResponse, SessionError> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT batch_id, batch_hash, server_received_at, payload_json FROM telemetry_batches WHERE session_id = $1 ORDER BY server_received_at DESC LIMIT 1",
                &[&authorized.session_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("telemetry batch not found".to_string()))?;
        let payload_json: String = row.get("payload_json");
        let payload: burd_protocol::TelemetryBatchPayload = serde_json::from_str(&payload_json)
            .map_err(|error| SessionError::Database(crate::db::DbError::new(error.to_string())))?;
        Ok(LatestTelemetryResponse {
            request_id: request_id.to_string(),
            session_id: authorized.session_id.clone(),
            batch_id: row.get("batch_id"),
            batch_hash: row.get("batch_hash"),
            server_received_at: row.get("server_received_at"),
            samples: payload.samples,
        })
    }

    pub async fn purge_expired_gpu_telemetry(
        &self,
        retention_days: u32,
    ) -> Result<u64, SessionError> {
        let client = self.connect().await?;
        let cutoff = (Utc::now() - Duration::days(i64::from(retention_days))).to_rfc3339();
        Ok(client
            .execute(
                "DELETE FROM telemetry_batches WHERE server_received_at < $1",
                &[&cutoff],
            )
            .await?)
    }
}

fn validate_batch_contract(
    signed: &SignedTelemetryBatch,
    message: &ClientControlMessage,
    authorized: &AuthorizedSession,
    policy: TelemetryPolicy,
) -> Result<(), SessionError> {
    let payload = &signed.payload;
    if payload.schema_version != TELEMETRY_SCHEMA_VERSION
        || signed.canonicalization_version != TELEMETRY_CANONICALIZATION_VERSION
    {
        return Err(SessionError::Invalid(
            "unsupported telemetry schema or canonicalization version".to_string(),
        ));
    }
    if payload.provider_id != authorized.provider_id
        || payload.device_id != authorized.device_id
        || payload.session_id != authorized.session_id
        || payload.control_sequence != message.sequence
    {
        return Err(SessionError::Unauthorized);
    }
    let maximum_samples = policy.max_samples_per_batch.min(256) as usize;
    if payload.samples.is_empty() || payload.samples.len() > maximum_samples {
        return Err(SessionError::Invalid(format!(
            "telemetry batch must contain between 1 and {} samples",
            maximum_samples
        )));
    }
    if payload.hardware_fingerprint.trim().is_empty()
        || payload.collector.trim().is_empty()
        || payload.collector.len() > 64
        || signed.public_key_id.trim().is_empty()
    {
        return Err(SessionError::Invalid(
            "telemetry batch identity fields are invalid".to_string(),
        ));
    }
    if message.sequence > i64::MAX as u64
        || payload.sample_sequence_end > i64::MAX as u64
        || payload.sample_sequence_start == 0
    {
        return Err(SessionError::Invalid(
            "telemetry sequence exceeds the supported range".to_string(),
        ));
    }
    let first = payload.samples.first().expect("samples checked nonempty");
    let last = payload.samples.last().expect("samples checked nonempty");
    if first.sample_sequence != payload.sample_sequence_start
        || last.sample_sequence != payload.sample_sequence_end
    {
        return Err(SessionError::Invalid(
            "telemetry sample range does not match the samples".to_string(),
        ));
    }
    for (offset, sample) in payload.samples.iter().enumerate() {
        let expected_sequence = payload
            .sample_sequence_start
            .checked_add(offset as u64)
            .ok_or_else(|| {
                SessionError::Invalid("telemetry sample sequence overflow".to_string())
            })?;
        if sample.sample_sequence != expected_sequence {
            return Err(SessionError::Invalid(
                "telemetry sample sequences must be contiguous".to_string(),
            ));
        }
        validate_sample(sample)?;
    }
    validate_observation_window(payload, policy.clock_skew_seconds)?;
    let expected_hash = telemetry_batch_hash(payload).map_err(SessionError::Invalid)?;
    if !constant_time_eq(expected_hash.as_bytes(), signed.batch_hash.as_bytes()) {
        return Err(SessionError::Invalid(
            "telemetry batch hash does not match the canonical payload".to_string(),
        ));
    }
    Ok(())
}

fn validate_sample(sample: &GpuTelemetrySample) -> Result<(), SessionError> {
    if sample.gpu_uuid.trim().is_empty()
        || sample.gpu_uuid.len() > 128
        || sample.gpu_name.trim().is_empty()
        || sample.gpu_name.len() > 256
        || sample.pci_bus_id.trim().is_empty()
        || sample.pci_bus_id.len() > 32
        || sample.driver_version.trim().is_empty()
        || sample.driver_version.len() > 64
        || sample.vram_total_mib == 0
        || sample.vram_total_mib > i64::MAX as u64
    {
        return Err(SessionError::Invalid(
            "telemetry sample is missing required GPU identity or VRAM".to_string(),
        ));
    }
    for (label, value) in [
        ("GPU utilization", sample.gpu_utilization_percent),
        ("memory utilization", sample.memory_utilization_percent),
    ] {
        if value.is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value)) {
            return Err(SessionError::Invalid(format!(
                "{label} must be between 0 and 100"
            )));
        }
    }
    if sample
        .temperature_celsius
        .is_some_and(|value| !value.is_finite() || !(-100.0..=200.0).contains(&value))
    {
        return Err(SessionError::Invalid(
            "GPU temperature is outside the accepted range".to_string(),
        ));
    }
    for value in [sample.power_draw_watts, sample.power_limit_watts] {
        if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(SessionError::Invalid(
                "GPU power values must be finite and nonnegative".to_string(),
            ));
        }
    }
    if sample
        .vram_used_mib
        .is_some_and(|used| used > sample.vram_total_mib)
        || sample
            .vram_free_mib
            .is_some_and(|free| free > sample.vram_total_mib)
    {
        return Err(SessionError::Invalid(
            "GPU VRAM values exceed total memory".to_string(),
        ));
    }
    if sample.processes.len() > 256
        || sample.processes.iter().any(|process| {
            process.process_name.len() > 128
                || process.process_name.is_empty()
                || process.process_name.contains('/')
                || process.process_name.contains('\\')
                || process.process_kind != "compute"
                || process
                    .used_gpu_memory_mib
                    .is_some_and(|used| used > sample.vram_total_mib)
        })
    {
        return Err(SessionError::Invalid(
            "GPU process telemetry is invalid or not redacted".to_string(),
        ));
    }
    if sample.throttle_reasons.len() > 16
        || sample
            .throttle_reasons
            .iter()
            .any(|reason| reason.is_empty() || reason.len() > 64)
        || sample
            .container_id
            .as_deref()
            .is_some_and(|value| value.len() > 128)
        || sample
            .job_id
            .as_deref()
            .is_some_and(|value| value.len() > 128)
    {
        return Err(SessionError::Invalid(
            "telemetry metadata exceeds accepted limits".to_string(),
        ));
    }
    Ok(())
}

fn validate_observation_window(
    payload: &burd_protocol::TelemetryBatchPayload,
    clock_skew_seconds: u32,
) -> Result<(), SessionError> {
    let start = parse_timestamp(&payload.collected_at_start)?;
    let end = parse_timestamp(&payload.collected_at_end)?;
    if start > end {
        return Err(SessionError::Invalid(
            "telemetry collection window is reversed".to_string(),
        ));
    }
    let now = Utc::now();
    let skew = Duration::seconds(i64::from(clock_skew_seconds));
    if start < now - skew || end > now + skew {
        return Err(SessionError::Invalid(
            "telemetry collection window is outside clock-skew tolerance".to_string(),
        ));
    }
    for sample in &payload.samples {
        let observed = parse_timestamp(&sample.observed_at)?;
        if observed < start || observed > end {
            return Err(SessionError::Invalid(
                "telemetry sample timestamp is outside the batch window".to_string(),
            ));
        }
    }
    Ok(())
}

async fn verify_batch_signature(
    transaction: &Transaction<'_>,
    authorized: &AuthorizedSession,
    signed: &SignedTelemetryBatch,
) -> Result<(), SessionError> {
    let row = transaction
        .query_opt(
            "SELECT public_key FROM provider_public_keys WHERE public_key_id = $1 AND provider_id = $2 AND device_id = $3 AND status = 'active'",
            &[&signed.public_key_id, &authorized.provider_id, &authorized.device_id],
        )
        .await?
        .ok_or(SessionError::Unauthorized)?;
    let public_key: String = row.get("public_key");
    let message = telemetry_batch_signature_message(
        &signed.payload,
        &signed.batch_hash,
        &signed.public_key_id,
    )
    .map_err(SessionError::Invalid)?;
    if !verify_message(&public_key, message.as_bytes(), &signed.signature).unwrap_or(false) {
        return Err(SessionError::SignatureInvalid);
    }
    Ok(())
}

async fn enforce_batch_interval(
    transaction: &Transaction<'_>,
    session_id: &str,
    minimum_seconds: u32,
) -> Result<(), SessionError> {
    let previous = transaction
        .query_opt(
            "SELECT server_received_at FROM telemetry_batches WHERE session_id = $1 ORDER BY server_received_at DESC LIMIT 1",
            &[&session_id],
        )
        .await?;
    if let Some(previous) = previous {
        let received_at: String = previous.get("server_received_at");
        let previous = parse_timestamp(&received_at)?;
        if Utc::now() - previous < Duration::seconds(i64::from(minimum_seconds)) {
            return Err(SessionError::Conflict(
                "telemetry batch frequency exceeds server policy".to_string(),
            ));
        }
    }
    Ok(())
}

async fn insert_sample(
    transaction: &Transaction<'_>,
    batch_id: &str,
    authorized: &AuthorizedSession,
    server_received_at: &str,
    sample: &GpuTelemetrySample,
) -> Result<(), SessionError> {
    let sample_id = format!("telemetry_sample_{}", Uuid::new_v4());
    let sample_json =
        serde_json::to_string(sample).map_err(|error| SessionError::Invalid(error.to_string()))?;
    transaction
        .execute(
            "INSERT INTO gpu_telemetry_samples (sample_id, batch_id, provider_id, device_id, session_id, sample_sequence, observed_at, server_received_at, gpu_uuid, pci_bus_id, gpu_utilization_percent, memory_utilization_percent, vram_used_mib, vram_total_mib, temperature_celsius, power_draw_watts, sample_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
            &[&sample_id, &batch_id, &authorized.provider_id, &authorized.device_id, &authorized.session_id, &(sample.sample_sequence as i64), &sample.observed_at, &server_received_at, &sample.gpu_uuid, &sample.pci_bus_id, &sample.gpu_utilization_percent, &sample.memory_utilization_percent, &sample.vram_used_mib.map(|value| value as i64), &(sample.vram_total_mib as i64), &sample.temperature_celsius, &sample.power_draw_watts, &sample_json],
        )
        .await?;
    Ok(())
}

fn parse_timestamp(raw: &str) -> Result<chrono::DateTime<Utc>, SessionError> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| SessionError::Invalid(format!("invalid telemetry timestamp: {error}")))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_paths_and_impossible_metrics_are_rejected() {
        let mut sample = sample();
        sample.processes.push(burd_protocol::GpuProcessTelemetry {
            pid: 7,
            process_name: "/private/job.py".to_string(),
            used_gpu_memory_mib: Some(1),
            process_kind: "compute".to_string(),
        });
        assert!(validate_sample(&sample).is_err());

        sample.processes.clear();
        sample.gpu_utilization_percent = Some(101.0);
        assert!(validate_sample(&sample).is_err());
    }

    #[test]
    fn overflowing_sample_ranges_are_rejected_without_panicking() {
        let mut first = sample();
        first.sample_sequence = u64::MAX;
        let mut second = sample();
        second.sample_sequence = 1;
        let now = Utc::now().to_rfc3339();
        let payload = burd_protocol::TelemetryBatchPayload {
            schema_version: TELEMETRY_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            control_sequence: 1,
            sample_sequence_start: u64::MAX,
            sample_sequence_end: 1,
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            collector: "fixture".to_string(),
            collected_at_start: now.clone(),
            collected_at_end: now,
            samples: vec![first, second],
        };
        let signed = SignedTelemetryBatch {
            batch_hash: telemetry_batch_hash(&payload).unwrap(),
            payload,
            public_key_id: "key_1".to_string(),
            signature: "invalid".to_string(),
            canonicalization_version: TELEMETRY_CANONICALIZATION_VERSION.to_string(),
        };
        let message = ClientControlMessage {
            session_id: "session_1".to_string(),
            device_id: "device_1".to_string(),
            sequence: 1,
            sent_at: Utc::now().to_rfc3339(),
            message_type: "telemetry_batch".to_string(),
            payload: serde_json::Value::Null,
        };
        let authorized = AuthorizedSession {
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            sequence_last: 0,
            heartbeat_interval_seconds: 15,
            missed_heartbeat_limit: 3,
        };
        assert!(
            validate_batch_contract(
                &signed,
                &message,
                &authorized,
                TelemetryPolicy {
                    max_samples_per_batch: 64,
                    min_batch_interval_seconds: 5,
                    clock_skew_seconds: 300,
                }
            )
            .is_err()
        );
    }

    fn sample() -> GpuTelemetrySample {
        GpuTelemetrySample {
            sample_sequence: 1,
            observed_at: Utc::now().to_rfc3339(),
            gpu_uuid: "GPU-test".to_string(),
            gpu_name: "NVIDIA RTX 4090".to_string(),
            pci_bus_id: "00000000:01:00.0".to_string(),
            pci_vendor_id: Some("10de".to_string()),
            pci_device_id: Some("2684".to_string()),
            compute_capability: Some("8.9".to_string()),
            driver_version: "576.80".to_string(),
            cuda_driver_version: Some("12.9".to_string()),
            cuda_runtime_version: None,
            vram_total_mib: 24564,
            vram_used_mib: Some(1000),
            vram_free_mib: Some(23564),
            gpu_utilization_percent: Some(20.0),
            memory_utilization_percent: Some(10.0),
            temperature_celsius: Some(50.0),
            power_draw_watts: Some(80.0),
            power_limit_watts: Some(450.0),
            graphics_clock_mhz: Some(2000),
            sm_clock_mhz: Some(2000),
            memory_clock_mhz: Some(10000),
            performance_state: Some("P2".to_string()),
            throttle_reasons: vec![],
            ecc_corrected_errors: None,
            ecc_uncorrected_errors: None,
            processes: vec![],
            container_id: None,
            job_id: None,
        }
    }
}
