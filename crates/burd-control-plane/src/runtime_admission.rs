use crate::Database;
use crate::db::{NewAuditEvent, insert_audit_event};
use crate::remote_session::{AuthorizedSession, SessionError};
use burd_protocol::{
    ListProviderRuntimeAdmissionsResponse, ProviderRuntimeObservationPayload,
    ProviderRuntimeVerificationRecord, RUNTIME_ADMISSION_SCHEMA_VERSION, RuntimeAdmissionDecision,
    SignedProviderRuntimeObservation, SubmitProviderRuntimeObservationResponse,
    provider_runtime_observation_hash, provider_runtime_observation_signature_message,
    runtime_admission_claims_from_observation, runtime_admission_fingerprint,
    validate_provider_runtime_verification_record, validate_runtime_admission_decision,
    validate_signed_provider_runtime_observation, verify_message,
};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashSet;
use uuid::Uuid;

const MAX_ADMISSION_GPUS: i64 = 200;

#[derive(Debug, Clone)]
pub struct RuntimeAdmissionPolicy {
    pub clock_skew_seconds: u32,
    pub observation_max_age_seconds: u32,
    pub approved_proof_image_ref: Option<String>,
}

#[derive(Debug, Clone)]
struct InventoryCandidate {
    device_id: String,
    device_status: String,
    gpu_uuid: String,
    gpu_status: String,
    inventory_public_key_id: String,
}

#[derive(Debug, Clone)]
struct ObservationSnapshot {
    hash: String,
    public_key_id: String,
    session_id: String,
    server_received_at: DateTime<Utc>,
    session_status: String,
    session_hardware_fingerprint: Option<String>,
    payload: ProviderRuntimeObservationPayload,
}

#[derive(Debug)]
struct AdmissionContext {
    provider_id: String,
    provider_status: String,
    inventory: InventoryCandidate,
    active_public_key_ids: Vec<String>,
    observation: Option<ObservationSnapshot>,
    observation_invalid: bool,
    verification: Option<ProviderRuntimeVerificationRecord>,
    verification_invalid: bool,
}

impl Database {
    pub async fn submit_provider_runtime_observation(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        session_id: &str,
        signed: &SignedProviderRuntimeObservation,
        policy: &RuntimeAdmissionPolicy,
    ) -> Result<SubmitProviderRuntimeObservationResponse, SessionError> {
        if session_id != authorized.session_id
            || signed.payload.provider_id != authorized.provider_id
            || signed.payload.device_id != authorized.device_id
            || signed.payload.session_id != authorized.session_id
        {
            return Err(SessionError::Unauthorized);
        }
        validate_signed_provider_runtime_observation(signed).map_err(SessionError::Invalid)?;
        let computed_hash =
            provider_runtime_observation_hash(&signed.payload).map_err(SessionError::Invalid)?;
        if computed_hash != signed.observation_hash {
            return Err(SessionError::Invalid(
                "runtime observation hash does not match its canonical payload".to_string(),
            ));
        }
        let observed_at = parse_time(&signed.payload.observed_at)?;
        let now = Utc::now();
        if observed_at > now + Duration::seconds(i64::from(policy.clock_skew_seconds)) {
            return Err(SessionError::Invalid(
                "runtime observation timestamp is in the future".to_string(),
            ));
        }
        if observed_at < now - Duration::seconds(i64::from(policy.observation_max_age_seconds)) {
            return Err(SessionError::Expired);
        }

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let session = transaction
            .query_opt(
                "SELECT s.status, s.hardware_fingerprint, p.status AS provider_status, d.status AS device_status FROM provider_sessions s JOIN providers p ON p.provider_id = s.provider_id JOIN devices d ON d.device_id = s.device_id AND d.provider_id = s.provider_id WHERE s.session_id = $1 AND s.provider_id = $2 AND s.device_id = $3 FOR UPDATE",
                &[&authorized.session_id, &authorized.provider_id, &authorized.device_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
        let session_status: String = session.get("status");
        let provider_status: String = session.get("provider_status");
        let device_status: String = session.get("device_status");
        let hardware_fingerprint: Option<String> = session.get("hardware_fingerprint");
        if !matches!(session_status.as_str(), "online" | "degraded")
            || matches!(provider_status.as_str(), "blocked" | "quarantined")
            || device_status != "active"
        {
            return Err(SessionError::Revoked);
        }
        if hardware_fingerprint.as_deref() != Some(signed.payload.hardware_fingerprint.as_str()) {
            return Err(SessionError::Conflict(
                "runtime observation hardware fingerprint changed".to_string(),
            ));
        }
        let public_key = transaction
            .query_opt(
                "SELECT public_key FROM provider_public_keys WHERE public_key_id = $1 AND provider_id = $2 AND device_id = $3 AND status = 'active'",
                &[&signed.public_key_id, &authorized.provider_id, &authorized.device_id],
            )
            .await?
            .map(|row| row.get::<_, String>("public_key"))
            .ok_or(SessionError::Unauthorized)?;
        let signature_message = provider_runtime_observation_signature_message(
            &signed.payload,
            &computed_hash,
            &signed.public_key_id,
        )
        .map_err(SessionError::Invalid)?;
        if !verify_message(&public_key, signature_message.as_bytes(), &signed.signature)
            .map_err(SessionError::Invalid)?
        {
            return Err(SessionError::SignatureInvalid);
        }

        let inventory_rows = transaction
            .query(
                "SELECT gpu_uuid, status, public_key_id FROM device_gpu_inventory WHERE provider_id = $1 AND device_id = $2 AND inventory_hash = (SELECT inventory_hash FROM device_gpu_inventory WHERE provider_id = $1 AND device_id = $2 ORDER BY server_received_at DESC, observed_at DESC LIMIT 1)",
                &[&authorized.provider_id, &authorized.device_id],
            )
            .await?;
        let active_inventory_gpus = inventory_rows
            .iter()
            .filter(|row| row.get::<_, String>("status") == "active")
            .map(|row| row.get::<_, String>("gpu_uuid").to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let observation_gpus = signed
            .payload
            .gpu_uuids
            .iter()
            .map(|value| value.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let inventory_key_matches = inventory_rows
            .iter()
            .all(|row| row.get::<_, String>("public_key_id") == signed.public_key_id);
        if active_inventory_gpus.is_empty()
            || active_inventory_gpus != observation_gpus
            || !inventory_key_matches
        {
            return Err(SessionError::Conflict(
                "runtime observation does not match the current signed GPU inventory".to_string(),
            ));
        }

        let server_received_at = now.to_rfc3339();
        let payload_json = serde_json::to_string(&signed.payload)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let observation_id = format!("runtime_observation_{}", Uuid::new_v4());
        let inserted = transaction
            .execute(
                "INSERT INTO provider_runtime_observations (observation_id, observation_hash, provider_id, device_id, session_id, public_key_id, signature, canonicalization_version, hardware_fingerprint, host_os, runtime_backend, observed_at, server_received_at, payload_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) ON CONFLICT (observation_hash) DO NOTHING",
                &[
                    &observation_id,
                    &computed_hash,
                    &authorized.provider_id,
                    &authorized.device_id,
                    &authorized.session_id,
                    &signed.public_key_id,
                    &signed.signature,
                    &signed.canonicalization_version,
                    &signed.payload.hardware_fingerprint,
                    &signed.payload.host_os,
                    &signed.payload.runtime_backend,
                    &signed.payload.observed_at,
                    &server_received_at,
                    &payload_json,
                ],
            )
            .await?;
        let duplicate = inserted == 0;
        if !duplicate {
            let metadata = serde_json::json!({
                "observation_hash": computed_hash,
                "runtime_backend": signed.payload.runtime_backend,
                "gpu_count": signed.payload.gpu_uuids.len(),
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "device_key",
                    actor_id: Some(signed.public_key_id.clone()),
                    entity_type: "provider_runtime_observation",
                    entity_id: &observation_id,
                    event_type: "provider_runtime_observation.accepted",
                    idempotency_key: None,
                    summary: "signed provider runtime observation accepted",
                    metadata_json: &metadata,
                },
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(SubmitProviderRuntimeObservationResponse {
            request_id: request_id.to_string(),
            observation_hash: computed_hash,
            duplicate,
            server_received_at,
        })
    }

    pub async fn list_provider_runtime_admissions(
        &self,
        request_id: &str,
        provider_id: &str,
        policy: &RuntimeAdmissionPolicy,
    ) -> Result<ListProviderRuntimeAdmissionsResponse, SessionError> {
        if !safe_id(provider_id) {
            return Err(SessionError::Invalid("provider_id is invalid".to_string()));
        }
        let now = Utc::now();
        let client = self.connect().await?;
        let provider_status = client
            .query_opt(
                "SELECT status FROM providers WHERE provider_id = $1",
                &[&provider_id],
            )
            .await?
            .map(|row| row.get::<_, String>("status"))
            .ok_or_else(|| SessionError::NotFound("provider not found".to_string()))?;
        let rows = client
            .query(
                "SELECT i.device_id, d.status AS device_status, i.gpu_uuid, i.status AS gpu_status, i.public_key_id FROM device_gpu_inventory i JOIN devices d ON d.device_id = i.device_id AND d.provider_id = i.provider_id WHERE i.provider_id = $1 AND i.inventory_hash = (SELECT latest.inventory_hash FROM device_gpu_inventory latest WHERE latest.provider_id = i.provider_id AND latest.device_id = i.device_id ORDER BY latest.server_received_at DESC, latest.observed_at DESC LIMIT 1) ORDER BY i.device_id, lower(i.gpu_uuid) LIMIT $2",
                &[&provider_id, &MAX_ADMISSION_GPUS],
            )
            .await?;
        let mut admissions = Vec::with_capacity(rows.len());
        for row in rows {
            let inventory = InventoryCandidate {
                device_id: row.get("device_id"),
                device_status: row.get("device_status"),
                gpu_uuid: row.get("gpu_uuid"),
                gpu_status: row.get("gpu_status"),
                inventory_public_key_id: row.get("public_key_id"),
            };
            let context =
                load_admission_context(&client, provider_id, provider_status.clone(), inventory)
                    .await?;
            let decision = evaluate_runtime_admission(context, policy, now);
            validate_runtime_admission_decision(&decision).map_err(SessionError::Invalid)?;
            admissions.push(decision);
        }
        admissions.sort_by(|left, right| {
            (&left.device_id, &left.gpu_uuid).cmp(&(&right.device_id, &right.gpu_uuid))
        });
        Ok(ListProviderRuntimeAdmissionsResponse {
            request_id: request_id.to_string(),
            provider_id: provider_id.to_string(),
            admissions,
        })
    }
}

async fn load_admission_context(
    client: &tokio_postgres::Client,
    provider_id: &str,
    provider_status: String,
    inventory: InventoryCandidate,
) -> Result<AdmissionContext, SessionError> {
    let key_rows = client
        .query(
            "SELECT public_key_id FROM provider_public_keys WHERE provider_id = $1 AND device_id = $2 AND status = 'active' ORDER BY created_at DESC",
            &[&provider_id, &inventory.device_id],
        )
        .await?;
    let active_public_key_ids = key_rows
        .into_iter()
        .map(|row| row.get("public_key_id"))
        .collect::<Vec<String>>();

    let observation_row = client
        .query_opt(
            "SELECT o.observation_hash, o.public_key_id, o.session_id, o.server_received_at, o.payload_json, s.status AS session_status, s.hardware_fingerprint AS session_hardware_fingerprint FROM provider_runtime_observations o JOIN provider_sessions s ON s.session_id = o.session_id WHERE o.provider_id = $1 AND o.device_id = $2 ORDER BY o.server_received_at DESC LIMIT 1",
            &[&provider_id, &inventory.device_id],
        )
        .await?;
    let mut observation_invalid = false;
    let observation = observation_row.and_then(|row| {
        let payload_json: String = row.get("payload_json");
        match serde_json::from_str::<ProviderRuntimeObservationPayload>(&payload_json) {
            Ok(payload) => match parse_time(&row.get::<_, String>("server_received_at")) {
                Ok(server_received_at) => Some(ObservationSnapshot {
                    hash: row.get("observation_hash"),
                    public_key_id: row.get("public_key_id"),
                    session_id: row.get("session_id"),
                    server_received_at,
                    session_status: row.get("session_status"),
                    session_hardware_fingerprint: row.get("session_hardware_fingerprint"),
                    payload,
                }),
                Err(_) => {
                    observation_invalid = true;
                    None
                }
            },
            Err(_) => {
                observation_invalid = true;
                None
            }
        }
    });

    let verification_row = client
        .query_opt(
            "SELECT record_json FROM provider_runtime_verifications WHERE provider_id = $1 AND device_id = $2 AND lower(gpu_uuid) = lower($3) ORDER BY verified_at DESC LIMIT 1",
            &[&provider_id, &inventory.device_id, &inventory.gpu_uuid],
        )
        .await?;
    let mut verification_invalid = false;
    let verification = verification_row.and_then(|row| {
        let record_json: String = row.get("record_json");
        match serde_json::from_str(&record_json) {
            Ok(record) => Some(record),
            Err(_) => {
                verification_invalid = true;
                None
            }
        }
    });

    Ok(AdmissionContext {
        provider_id: provider_id.to_string(),
        provider_status,
        inventory,
        active_public_key_ids,
        observation,
        observation_invalid,
        verification,
        verification_invalid,
    })
}

fn evaluate_runtime_admission(
    context: AdmissionContext,
    policy: &RuntimeAdmissionPolicy,
    now: DateTime<Utc>,
) -> RuntimeAdmissionDecision {
    let mut reasons = Vec::new();
    if matches!(context.provider_status.as_str(), "blocked" | "quarantined") {
        reasons.push("provider_not_active".to_string());
    }
    if context.inventory.device_status != "active" {
        reasons.push("device_not_active".to_string());
    }
    if context.inventory.gpu_status != "active" {
        reasons.push("gpu_not_active".to_string());
    }
    let active_key = if context.active_public_key_ids.len() == 1 {
        context.active_public_key_ids.first()
    } else {
        reasons.push(if context.active_public_key_ids.is_empty() {
            "active_device_key_missing".to_string()
        } else {
            "active_device_key_ambiguous".to_string()
        });
        None
    };
    if active_key != Some(&context.inventory.inventory_public_key_id) {
        reasons.push("inventory_key_changed".to_string());
    }
    if context.observation_invalid {
        reasons.push("runtime_observation_invalid".to_string());
    }
    if context.verification_invalid {
        reasons.push("runtime_verification_invalid".to_string());
    }
    let mut runtime_backend = None;
    let mut observation_hash = None;
    if let Some(observation) = context.observation.as_ref() {
        runtime_backend = Some(observation.payload.runtime_backend.clone());
        observation_hash = Some(observation.hash.clone());
        let payload_hash_matches = provider_runtime_observation_hash(&observation.payload)
            .is_ok_and(|value| value == observation.hash);
        if !payload_hash_matches
            || observation.payload.provider_id != context.provider_id
            || observation.payload.device_id != context.inventory.device_id
            || observation.payload.session_id != observation.session_id
        {
            reasons.push("runtime_observation_invalid".to_string());
        }
        match parse_time(&observation.payload.observed_at) {
            Ok(observed_at)
                if observed_at
                    < now - Duration::seconds(i64::from(policy.observation_max_age_seconds)) =>
            {
                reasons.push("runtime_observation_stale".to_string());
            }
            Ok(observed_at)
                if observed_at > now + Duration::seconds(i64::from(policy.clock_skew_seconds)) =>
            {
                reasons.push("runtime_observation_timestamp_invalid".to_string());
            }
            Ok(_) => {}
            Err(_) => reasons.push("runtime_observation_timestamp_invalid".to_string()),
        }
        if now - observation.server_received_at
            > Duration::seconds(i64::from(policy.observation_max_age_seconds))
        {
            reasons.push("runtime_observation_stale".to_string());
        }
        if active_key != Some(&observation.public_key_id) {
            reasons.push("runtime_observation_key_changed".to_string());
        }
        if !matches!(observation.session_status.as_str(), "online" | "degraded") {
            reasons.push("runtime_observation_session_not_online".to_string());
        }
        if observation.session_hardware_fingerprint.as_deref()
            != Some(observation.payload.hardware_fingerprint.as_str())
        {
            reasons.push("session_hardware_changed".to_string());
        }
        if !observation
            .payload
            .gpu_uuids
            .iter()
            .any(|value| value.eq_ignore_ascii_case(&context.inventory.gpu_uuid))
        {
            reasons.push("gpu_missing_from_runtime".to_string());
        }
        if observation.payload.host_os == "windows" {
            reasons.push("windows_physical_gate_required".to_string());
        }
    } else if !context.observation_invalid {
        reasons.push("runtime_observation_missing".to_string());
    }

    let mut verification_id = None;
    let mut verification_fingerprint = None;
    if let Some(verification) = context.verification.as_ref() {
        verification_id = Some(verification.verification_id.clone());
        verification_fingerprint = Some(verification.runtime_verification_fingerprint.clone());
        if verification.provider_id != context.provider_id
            || verification.device_id != context.inventory.device_id
        {
            reasons.push("runtime_verification_identity_changed".to_string());
        }
        if verification.status != "verified" {
            reasons.push("runtime_verification_not_verified".to_string());
        }
        let verification_unexpired = match parse_time(&verification.expires_at) {
            Ok(expires_at) if expires_at <= now => {
                reasons.push("runtime_verification_expired".to_string());
                false
            }
            Ok(_) => true,
            Err(_) => {
                reasons.push("runtime_verification_invalid".to_string());
                false
            }
        };
        if verification.status == "verified"
            && verification_unexpired
            && validate_provider_runtime_verification_record(verification, now).is_err()
        {
            reasons.push("runtime_verification_invalid".to_string());
        }
        if verification.public_key_id.as_ref() != active_key {
            reasons.push("runtime_verification_key_changed".to_string());
        }
        if !verification
            .gpu_uuid
            .eq_ignore_ascii_case(&context.inventory.gpu_uuid)
        {
            reasons.push("runtime_verification_gpu_changed".to_string());
        }
        match policy.approved_proof_image_ref.as_deref() {
            None => reasons.push("runtime_proof_policy_unconfigured".to_string()),
            Some(approved) if verification.proof_image_digest != approved => {
                reasons.push("runtime_proof_image_changed".to_string());
            }
            Some(_) => {}
        }
        if let Some(observation) = context.observation.as_ref() {
            if verification.hardware_fingerprint != observation.payload.hardware_fingerprint {
                reasons.push("hardware_fingerprint_changed".to_string());
            }
            if verification.runtime_backend != observation.payload.runtime_backend {
                reasons.push("runtime_backend_changed".to_string());
            }
            match runtime_admission_claims_from_observation(
                &observation.payload,
                &context.inventory.gpu_uuid,
            )
            .and_then(|claims| runtime_admission_fingerprint(&claims))
            {
                Ok(current_fingerprint)
                    if verification.runtime_admission_fingerprint.as_deref()
                        != Some(current_fingerprint.as_str()) =>
                {
                    reasons.push("runtime_changed".to_string());
                }
                Ok(_) => {}
                Err(_) => reasons.push("runtime_observation_invalid".to_string()),
            }
        }
    } else if !context.verification_invalid {
        reasons.push("runtime_verification_required".to_string());
    }

    reasons.sort();
    reasons.dedup();
    RuntimeAdmissionDecision {
        schema_version: RUNTIME_ADMISSION_SCHEMA_VERSION.to_string(),
        provider_id: context.provider_id,
        device_id: context.inventory.device_id,
        gpu_uuid: context.inventory.gpu_uuid,
        status: if reasons.is_empty() {
            "admitted".to_string()
        } else {
            "denied".to_string()
        },
        reason_codes: reasons,
        runtime_backend,
        verification_id,
        runtime_verification_fingerprint: verification_fingerprint,
        runtime_observation_hash: observation_hash,
        evaluated_at: now.to_rfc3339(),
    }
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, SessionError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| SessionError::Invalid("runtime admission timestamp is invalid".to_string()))
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{
        AGENT_RUNTIME_CONTRACT_VERSION, DEVICE_GPU_INVENTORY_CANONICALIZATION_VERSION,
        DEVICE_GPU_INVENTORY_SCHEMA_VERSION, DeviceGpuInventoryGpu, DeviceGpuInventoryPayload,
        RUNTIME_PROOF_POLICY_VERSION, RUNTIME_VERIFICATION_CANONICALIZATION_VERSION,
        RUNTIME_VERIFICATION_RECORD_SCHEMA_VERSION, RuntimeAdmissionFingerprintClaims,
        SignedDeviceGpuInventory, device_gpu_inventory_hash,
        device_gpu_inventory_signature_message, generate_keypair,
        provider_runtime_observation_hash, provider_runtime_observation_signature_message,
        runtime_admission_claims_from_observation, sign_message,
    };

    fn proof_image() -> String {
        format!("ghcr.io/burd/runtime-proof@sha256:{}", "a".repeat(64))
    }

    fn policy() -> RuntimeAdmissionPolicy {
        RuntimeAdmissionPolicy {
            clock_skew_seconds: 300,
            observation_max_age_seconds: 180,
            approved_proof_image_ref: Some(proof_image()),
        }
    }

    fn observation(now: DateTime<Utc>) -> ProviderRuntimeObservationPayload {
        ProviderRuntimeObservationPayload {
            schema_version: burd_protocol::PROVIDER_RUNTIME_OBSERVATION_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_reconnected".to_string(),
            hardware_fingerprint: "a".repeat(64),
            host_os: "linux".to_string(),
            runtime_backend: "docker_linux_native".to_string(),
            container_os: "linux".to_string(),
            gpu_backend: "cuda".to_string(),
            gpu_runtime: "nvidia".to_string(),
            isolation_mode: "linux_container".to_string(),
            docker_server_version: "28.3.0".to_string(),
            nvidia_driver_version: "580.1".to_string(),
            nvidia_runtime: "nvidia".to_string(),
            gpu_uuids: vec!["GPU-A".to_string()],
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            observed_at: now.to_rfc3339(),
        }
    }

    fn verification(
        observation: &ProviderRuntimeObservationPayload,
        now: DateTime<Utc>,
    ) -> ProviderRuntimeVerificationRecord {
        let claims: RuntimeAdmissionFingerprintClaims =
            runtime_admission_claims_from_observation(observation, "GPU-A").unwrap();
        let admission_fingerprint = runtime_admission_fingerprint(&claims).unwrap();
        ProviderRuntimeVerificationRecord {
            schema_version: RUNTIME_VERIFICATION_RECORD_SCHEMA_VERSION.to_string(),
            verification_id: "runtime_verification_1".to_string(),
            challenge_id: "runtime_challenge_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            // Challenge is session-bound, but the resulting verification survives reconnects.
            session_id: "session_original".to_string(),
            hardware_fingerprint: observation.hardware_fingerprint.clone(),
            gpu_uuid: "GPU-A".to_string(),
            host_os: observation.host_os.clone(),
            runtime_backend: observation.runtime_backend.clone(),
            status: "verified".to_string(),
            gpu_uuid_binding: "verified".to_string(),
            runtime_verification_fingerprint: "b".repeat(64),
            proof_policy_version: RUNTIME_PROOF_POLICY_VERSION.to_string(),
            agent_runtime_contract_version: AGENT_RUNTIME_CONTRACT_VERSION.to_string(),
            proof_image_digest: proof_image(),
            public_key_id: Some("key_1".to_string()),
            runtime_admission_fingerprint: Some(admission_fingerprint),
            runtime_admission_claims: Some(claims),
            verified_at: (now - Duration::seconds(60)).to_rfc3339(),
            expires_at: (now + Duration::hours(1)).to_rfc3339(),
            reason_codes: Vec::new(),
        }
    }

    fn context(now: DateTime<Utc>) -> AdmissionContext {
        let payload = observation(now);
        let observation_hash = provider_runtime_observation_hash(&payload).unwrap();
        AdmissionContext {
            provider_id: "provider_1".to_string(),
            provider_status: "available".to_string(),
            inventory: InventoryCandidate {
                device_id: "device_1".to_string(),
                device_status: "active".to_string(),
                gpu_uuid: "GPU-A".to_string(),
                gpu_status: "active".to_string(),
                inventory_public_key_id: "key_1".to_string(),
            },
            active_public_key_ids: vec!["key_1".to_string()],
            observation: Some(ObservationSnapshot {
                hash: observation_hash,
                public_key_id: "key_1".to_string(),
                session_id: "session_reconnected".to_string(),
                server_received_at: now,
                session_status: "online".to_string(),
                session_hardware_fingerprint: Some(payload.hardware_fingerprint.clone()),
                payload: payload.clone(),
            }),
            observation_invalid: false,
            verification: Some(verification(&payload, now)),
            verification_invalid: false,
        }
    }

    #[test]
    fn admits_same_device_runtime_after_session_reconnect() {
        let now = Utc::now();
        let decision = evaluate_runtime_admission(context(now), &policy(), now);
        assert_eq!(decision.status, "admitted");
        assert!(decision.reason_codes.is_empty());
    }

    #[test]
    fn driver_drift_invalidates_admission() {
        let now = Utc::now();
        let mut context = context(now);
        context
            .observation
            .as_mut()
            .unwrap()
            .payload
            .nvidia_driver_version = "581.0".to_string();
        let decision = evaluate_runtime_admission(context, &policy(), now);
        assert!(
            decision
                .reason_codes
                .contains(&"runtime_changed".to_string())
        );
    }

    #[test]
    fn key_rotation_requires_new_inventory_observation_and_verification() {
        let now = Utc::now();
        let mut context = context(now);
        context.active_public_key_ids = vec!["key_2".to_string()];
        let decision = evaluate_runtime_admission(context, &policy(), now);
        assert!(
            decision
                .reason_codes
                .contains(&"inventory_key_changed".to_string())
        );
        assert!(
            decision
                .reason_codes
                .contains(&"runtime_verification_key_changed".to_string())
        );
        assert!(
            decision
                .reason_codes
                .contains(&"runtime_observation_key_changed".to_string())
        );
    }

    #[test]
    fn expired_verification_is_denied_with_specific_reason() {
        let now = Utc::now();
        let mut context = context(now);
        context.verification.as_mut().unwrap().verified_at =
            (now - Duration::hours(2)).to_rfc3339();
        context.verification.as_mut().unwrap().expires_at = (now - Duration::hours(1)).to_rfc3339();
        let decision = evaluate_runtime_admission(context, &policy(), now);
        assert!(
            decision
                .reason_codes
                .contains(&"runtime_verification_expired".to_string())
        );
    }

    #[test]
    fn windows_stays_denied_until_the_physical_gate_passes() {
        let now = Utc::now();
        let mut context = context(now);
        let observation = context.observation.as_mut().unwrap();
        observation.payload.host_os = "windows".to_string();
        observation.payload.runtime_backend = "docker_wsl2".to_string();
        let decision = evaluate_runtime_admission(context, &policy(), now);
        assert!(
            decision
                .reason_codes
                .contains(&"windows_physical_gate_required".to_string())
        );
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_admission_recovers_after_complete_key_rotation_refresh() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_runtime_admission_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let session_expires_at = (now + Duration::hours(2)).to_rfc3339();
        let keys = generate_keypair().unwrap();
        let client = db.connect().await.unwrap();
        client
            .execute(
                "INSERT INTO providers (provider_id, status, created_at, updated_at) VALUES ('provider_1', 'available', $1, $1)",
                &[&now_text],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ('device_1', 'provider_1', 'machine_1', 'active', $1, $1)",
                &[&now_text],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at) VALUES ('key_1', 'provider_1', 'device_1', $1, 'ed25519', 'active', $2)",
                &[&keys.public_key_base64, &now_text],
            )
            .await
            .unwrap();
        for (session_id, status) in [
            ("session_original", "expired"),
            ("session_reconnected", "online"),
        ] {
            client
                .execute(
                    "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint) VALUES ($1, 'provider_1', 'device_1', $2, 0, $3, $4, $5)",
                    &[&session_id, &status, &now_text, &session_expires_at, &"a".repeat(64)],
                )
                .await
                .unwrap();
        }
        client
            .execute(
                "INSERT INTO device_gpu_inventory (inventory_row_id, provider_id, device_id, session_id, schema_version, inventory_hash, public_key_id, signature, canonicalization_version, gpu_uuid, gpu_index, backend, pci_vendor_id, pci_device_id, vram_total_mib, status, observed_at, server_received_at, payload_json, verification_json) VALUES ('inventory_1', 'provider_1', 'device_1', 'session_reconnected', 'burd-device-gpu-inventory-v1', 'inventory_hash_1', 'key_1', 'signature', 'burd-json-c14n-v1', 'GPU-A', 0, 'cuda', '10de', '2684', 24576, 'active', $1, $1, '{}', '{}')",
                &[&now_text],
            )
            .await
            .unwrap();

        let payload = observation(now);
        let record = verification(&payload, now);
        let record_json = serde_json::to_string(&record).unwrap();
        let admission_claims_json =
            serde_json::to_string(record.runtime_admission_claims.as_ref().unwrap()).unwrap();
        let challenge_json = serde_json::json!({
            "challenge_id": record.challenge_id,
            "session_id": record.session_id,
        })
        .to_string();
        client
            .execute(
                "INSERT INTO runtime_verification_challenges (challenge_id, provider_id, device_id, session_id, gpu_uuid, runtime_backend, hardware_fingerprint, status, nonce, challenge_json, verification_ttl_seconds, issued_at, expires_at, verified_at, public_key_id) VALUES ('runtime_challenge_1', 'provider_1', 'device_1', 'session_original', 'GPU-A', 'docker_linux_native', $1, 'verified', 'runtime_nonce_1', $2, 3600, $3, $4, $3, 'key_1')",
                &[&record.hardware_fingerprint, &challenge_json, &record.verified_at, &record.expires_at],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_runtime_verifications (verification_id, challenge_id, provider_id, device_id, session_id, gpu_uuid, runtime_backend, hardware_fingerprint, runtime_verification_fingerprint, status, verified_at, expires_at, record_json, public_key_id, runtime_admission_fingerprint, runtime_admission_claims_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'verified', $10, $11, $12, $13, $14, $15)",
                &[
                    &record.verification_id,
                    &record.challenge_id,
                    &record.provider_id,
                    &record.device_id,
                    &record.session_id,
                    &record.gpu_uuid,
                    &record.runtime_backend,
                    &record.hardware_fingerprint,
                    &record.runtime_verification_fingerprint,
                    &record.verified_at,
                    &record.expires_at,
                    &record_json,
                    &record.public_key_id.as_ref().unwrap(),
                    &record.runtime_admission_fingerprint.as_ref().unwrap(),
                    &admission_claims_json,
                ],
            )
            .await
            .unwrap();
        drop(client);

        let observation_hash = provider_runtime_observation_hash(&payload).unwrap();
        let message =
            provider_runtime_observation_signature_message(&payload, &observation_hash, "key_1")
                .unwrap();
        let signed = SignedProviderRuntimeObservation {
            payload,
            observation_hash,
            public_key_id: "key_1".to_string(),
            signature: sign_message(&keys.secret_key_base64, message.as_bytes()).unwrap(),
            canonicalization_version: RUNTIME_VERIFICATION_CANONICALIZATION_VERSION.to_string(),
        };
        let authorized = AuthorizedSession {
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_reconnected".to_string(),
            sequence_last: 0,
            heartbeat_interval_seconds: 15,
            missed_heartbeat_limit: 3,
        };
        let submitted = db
            .submit_provider_runtime_observation(
                "req_observation_1",
                &authorized,
                "session_reconnected",
                &signed,
                &policy(),
            )
            .await
            .unwrap();
        assert!(!submitted.duplicate);
        let duplicate = db
            .submit_provider_runtime_observation(
                "req_observation_2",
                &authorized,
                "session_reconnected",
                &signed,
                &policy(),
            )
            .await
            .unwrap();
        assert!(duplicate.duplicate);
        let decisions = db
            .list_provider_runtime_admissions("req_admission_1", "provider_1", &policy())
            .await
            .unwrap();
        assert_eq!(decisions.admissions.len(), 1);
        assert_eq!(decisions.admissions[0].status, "admitted");

        let rotated = generate_keypair().unwrap();
        let client = db.connect().await.unwrap();
        client
            .execute(
                "UPDATE provider_public_keys SET status = 'revoked', revoked_at = $1 WHERE public_key_id = 'key_1'",
                &[&Utc::now().to_rfc3339()],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at) VALUES ('key_2', 'provider_1', 'device_1', $1, 'ed25519', 'active', $2)",
                &[&rotated.public_key_base64, &Utc::now().to_rfc3339()],
            )
            .await
            .unwrap();
        drop(client);
        let decisions = db
            .list_provider_runtime_admissions("req_admission_2", "provider_1", &policy())
            .await
            .unwrap();
        assert_eq!(decisions.admissions[0].status, "denied");
        assert!(
            decisions.admissions[0]
                .reason_codes
                .contains(&"runtime_verification_key_changed".to_string())
        );
        assert!(
            decisions.admissions[0]
                .reason_codes
                .contains(&"inventory_key_changed".to_string())
        );
        assert!(
            decisions.admissions[0]
                .reason_codes
                .contains(&"runtime_observation_key_changed".to_string())
        );

        let recovery_now = Utc::now();
        let inventory_payload = DeviceGpuInventoryPayload {
            schema_version: DEVICE_GPU_INVENTORY_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_reconnected".to_string(),
            hardware_fingerprint: "a".repeat(64),
            observed_at: recovery_now.to_rfc3339(),
            gpus: vec![DeviceGpuInventoryGpu {
                gpu_uuid: "GPU-A".to_string(),
                gpu_index: 0,
                backend: "cuda".to_string(),
                pci_vendor_id: "10de".to_string(),
                pci_device_id: "2684".to_string(),
                vram_total_mib: Some(24_576),
                status: "active".to_string(),
            }],
        };
        let inventory_hash = device_gpu_inventory_hash(&inventory_payload).unwrap();
        let inventory_message =
            device_gpu_inventory_signature_message(&inventory_payload, &inventory_hash, "key_2")
                .unwrap();
        let rotated_inventory = SignedDeviceGpuInventory {
            payload: inventory_payload,
            inventory_hash,
            public_key_id: "key_2".to_string(),
            signature: sign_message(&rotated.secret_key_base64, inventory_message.as_bytes())
                .unwrap(),
            canonicalization_version: DEVICE_GPU_INVENTORY_CANONICALIZATION_VERSION.to_string(),
        };

        let rotated_observation_payload = observation(recovery_now);
        let rotated_observation_hash =
            provider_runtime_observation_hash(&rotated_observation_payload).unwrap();
        let rotated_observation_message = provider_runtime_observation_signature_message(
            &rotated_observation_payload,
            &rotated_observation_hash,
            "key_2",
        )
        .unwrap();
        let rotated_observation = SignedProviderRuntimeObservation {
            payload: rotated_observation_payload.clone(),
            observation_hash: rotated_observation_hash,
            public_key_id: "key_2".to_string(),
            signature: sign_message(
                &rotated.secret_key_base64,
                rotated_observation_message.as_bytes(),
            )
            .unwrap(),
            canonicalization_version: RUNTIME_VERIFICATION_CANONICALIZATION_VERSION.to_string(),
        };
        let rejected_before_inventory = db
            .submit_provider_runtime_observation(
                "req_observation_rotated_before_inventory",
                &authorized,
                "session_reconnected",
                &rotated_observation,
                &policy(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            rejected_before_inventory,
            SessionError::Conflict(message)
                if message == "runtime observation does not match the current signed GPU inventory"
        ));

        let refreshed_inventory = db
            .submit_device_gpu_inventory("req_inventory_rotated", &authorized, &rotated_inventory)
            .await
            .unwrap();
        assert!(!refreshed_inventory.duplicate);
        let refreshed_observation = db
            .submit_provider_runtime_observation(
                "req_observation_rotated",
                &authorized,
                "session_reconnected",
                &rotated_observation,
                &policy(),
            )
            .await
            .unwrap();
        assert!(!refreshed_observation.duplicate);

        let decisions = db
            .list_provider_runtime_admissions("req_admission_3", "provider_1", &policy())
            .await
            .unwrap();
        assert_eq!(decisions.admissions[0].status, "denied");
        assert!(
            decisions.admissions[0]
                .reason_codes
                .contains(&"runtime_verification_key_changed".to_string())
        );

        let mut rotated_record = verification(&rotated_observation_payload, recovery_now);
        rotated_record.verification_id = "runtime_verification_2".to_string();
        rotated_record.challenge_id = "runtime_challenge_2".to_string();
        rotated_record.session_id = "session_reconnected".to_string();
        rotated_record.runtime_verification_fingerprint = "c".repeat(64);
        rotated_record.public_key_id = Some("key_2".to_string());
        let rotated_record_json = serde_json::to_string(&rotated_record).unwrap();
        let rotated_claims_json =
            serde_json::to_string(rotated_record.runtime_admission_claims.as_ref().unwrap())
                .unwrap();
        let rotated_challenge_json = serde_json::json!({
            "challenge_id": rotated_record.challenge_id,
            "session_id": rotated_record.session_id,
        })
        .to_string();
        let client = db.connect().await.unwrap();
        client
            .execute(
                "UPDATE provider_runtime_verifications SET status = 'superseded' WHERE provider_id = 'provider_1' AND device_id = 'device_1' AND gpu_uuid = 'GPU-A' AND status = 'verified'",
                &[],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO runtime_verification_challenges (challenge_id, provider_id, device_id, session_id, gpu_uuid, runtime_backend, hardware_fingerprint, status, nonce, challenge_json, verification_ttl_seconds, issued_at, expires_at, verified_at, public_key_id) VALUES ('runtime_challenge_2', 'provider_1', 'device_1', 'session_reconnected', 'GPU-A', 'docker_linux_native', $1, 'verified', 'runtime_nonce_2', $2, 3600, $3, $4, $3, 'key_2')",
                &[&rotated_record.hardware_fingerprint, &rotated_challenge_json, &rotated_record.verified_at, &rotated_record.expires_at],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_runtime_verifications (verification_id, challenge_id, provider_id, device_id, session_id, gpu_uuid, runtime_backend, hardware_fingerprint, runtime_verification_fingerprint, status, verified_at, expires_at, record_json, public_key_id, runtime_admission_fingerprint, runtime_admission_claims_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'verified', $10, $11, $12, $13, $14, $15)",
                &[
                    &rotated_record.verification_id,
                    &rotated_record.challenge_id,
                    &rotated_record.provider_id,
                    &rotated_record.device_id,
                    &rotated_record.session_id,
                    &rotated_record.gpu_uuid,
                    &rotated_record.runtime_backend,
                    &rotated_record.hardware_fingerprint,
                    &rotated_record.runtime_verification_fingerprint,
                    &rotated_record.verified_at,
                    &rotated_record.expires_at,
                    &rotated_record_json,
                    &rotated_record.public_key_id.as_ref().unwrap(),
                    &rotated_record.runtime_admission_fingerprint.as_ref().unwrap(),
                    &rotated_claims_json,
                ],
            )
            .await
            .unwrap();
        drop(client);

        let decisions = db
            .list_provider_runtime_admissions("req_admission_4", "provider_1", &policy())
            .await
            .unwrap();
        assert_eq!(decisions.admissions.len(), 1);
        assert_eq!(decisions.admissions[0].status, "admitted");
        assert!(decisions.admissions[0].reason_codes.is_empty());
        db.drop_schema_for_test().await.unwrap();
    }
}
