use crate::db::{Database, NewAuditEvent, insert_audit_event};
use crate::remote_session::{AuthorizedSession, SessionError};
use burd_protocol::{
    ListProviderSecurityPosturesResponse, SECURITY_POLICY_VERSION,
    SECURITY_POSTURE_CANONICALIZATION_VERSION, SECURITY_POSTURE_SCHEMA_VERSION,
    SecurityPolicyStatusResponse, SecurityPostureRecord, SecurityPostureVerification,
    SignedSecurityPosture, SubmitSecurityPostureResponse, security_posture_hash,
    security_posture_signature_message, verify_message,
};
use chrono::Utc;
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityPolicy {
    pub min_agent_version: Option<String>,
    pub require_signed_agent_release: bool,
    pub require_hardware_backed_key: bool,
    pub require_remote_attestation: bool,
    pub require_sbom_hash: bool,
    pub accepted_release_channels: Vec<String>,
    pub accepted_attestation_modes: Vec<String>,
}

impl SecurityPolicy {
    pub fn status_response(&self, request_id: String) -> SecurityPolicyStatusResponse {
        SecurityPolicyStatusResponse {
            request_id,
            policy_version: SECURITY_POLICY_VERSION.to_string(),
            min_agent_version: self.min_agent_version.clone(),
            require_signed_agent_release: self.require_signed_agent_release,
            require_hardware_backed_key: self.require_hardware_backed_key,
            require_remote_attestation: self.require_remote_attestation,
            require_sbom_hash: self.require_sbom_hash,
            accepted_release_channels: self.accepted_release_channels.clone(),
            accepted_attestation_modes: self.accepted_attestation_modes.clone(),
        }
    }
}

impl Database {
    pub async fn submit_security_posture(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        signed: &SignedSecurityPosture,
        policy: SecurityPolicy,
    ) -> Result<SubmitSecurityPostureResponse, SessionError> {
        validate_signed_posture_shape(signed, authorized)?;
        let computed_hash =
            security_posture_hash(&signed.payload).map_err(SessionError::Invalid)?;
        let server_received_at = Utc::now().to_rfc3339();

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let context = load_security_context(&transaction, authorized, signed).await?;
        let verification = verify_security_posture(signed, &computed_hash, &context, &policy);
        if !verification.posture_hash_valid
            || !verification.active_key_bound
            || !verification.signature_valid
            || !verification.session_bound
            || !verification.fingerprint_bound
        {
            record_rejected_security_posture(
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

        if let Some(existing) = transaction
            .query_opt(
                &format!(
                    "{} WHERE posture_hash = $1",
                    security_posture_select_columns()
                ),
                &[&computed_hash],
            )
            .await?
        {
            let posture = security_posture_from_row(existing)?;
            transaction.commit().await?;
            return Ok(SubmitSecurityPostureResponse {
                request_id: request_id.to_string(),
                duplicate: true,
                posture,
            });
        }

        let posture_id = format!("security_posture_{}", Uuid::new_v4());
        let status = posture_status(&verification);
        let payload_json = serde_json::to_string(&signed.payload)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let verification_json = serde_json::to_string(&verification)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO device_security_postures (posture_id, provider_id, device_id, session_id, schema_version, policy_version, status, posture_hash, public_key_id, signature, canonicalization_version, agent_version, release_channel, key_storage_backend, key_hardware_backed, private_key_exportable, attestation_mode, attestation_evidence_hash, binary_hash, sbom_hash, vulnerability_scan_status, dependency_scan_status, hardware_fingerprint, observed_at, server_received_at, payload_json, verification_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27)",
                &[
                    &posture_id,
                    &authorized.provider_id,
                    &authorized.device_id,
                    &authorized.session_id,
                    &signed.payload.schema_version,
                    &SECURITY_POLICY_VERSION,
                    &status,
                    &computed_hash,
                    &signed.public_key_id,
                    &signed.signature,
                    &signed.canonicalization_version,
                    &signed.payload.agent_version,
                    &signed.payload.release.release_channel,
                    &signed.payload.key_storage.storage_backend,
                    &signed.payload.key_storage.hardware_backed,
                    &signed.payload.key_storage.private_key_exportable,
                    &signed.payload.attestation.mode,
                    &signed.payload.attestation.evidence_hash,
                    &signed.payload.release.binary_hash,
                    &signed.payload.artifact_integrity.sbom_hash,
                    &signed.payload.artifact_integrity.vulnerability_scan_status,
                    &signed.payload.artifact_integrity.dependency_scan_status,
                    &signed.payload.hardware_fingerprint,
                    &signed.payload.observed_at,
                    &server_received_at,
                    &payload_json,
                    &verification_json,
                ],
            )
            .await?;
        let audit_metadata = serde_json::json!({
            "posture_hash": computed_hash,
            "status": status,
            "release_channel": signed.payload.release.release_channel,
            "key_storage_backend": signed.payload.key_storage.storage_backend,
            "attestation_mode": signed.payload.attestation.mode,
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "device_key",
                actor_id: Some(signed.public_key_id.clone()),
                entity_type: "device_security_posture",
                entity_id: &posture_id,
                event_type: "security_posture.accepted",
                idempotency_key: None,
                summary: "signed security posture accepted",
                metadata_json: &audit_metadata,
            },
        )
        .await?;
        let row = transaction
            .query_one(
                &format!(
                    "{} WHERE posture_id = $1",
                    security_posture_select_columns()
                ),
                &[&posture_id],
            )
            .await?;
        let posture = security_posture_from_row(row)?;
        transaction.commit().await?;

        Ok(SubmitSecurityPostureResponse {
            request_id: request_id.to_string(),
            duplicate: false,
            posture,
        })
    }

    pub async fn list_provider_security_postures(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListProviderSecurityPosturesResponse, SessionError> {
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
        let limit = limit.clamp(1, 200) as i64;
        let rows = client
            .query(
                &format!(
                    "{} WHERE provider_id = $1 ORDER BY server_received_at DESC LIMIT $2",
                    security_posture_select_columns()
                ),
                &[&provider_id, &limit],
            )
            .await?;
        let records = rows
            .into_iter()
            .map(security_posture_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListProviderSecurityPosturesResponse {
            request_id: request_id.to_string(),
            provider_id: provider_id.to_string(),
            records,
        })
    }
}

#[derive(Debug)]
struct SecurityContext {
    session_status: Option<String>,
    session_fingerprint: Option<String>,
    active_public_key: Option<String>,
}

async fn load_security_context(
    transaction: &Transaction<'_>,
    authorized: &AuthorizedSession,
    signed: &SignedSecurityPosture,
) -> Result<SecurityContext, SessionError> {
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
    Ok(SecurityContext {
        session_status: session.as_ref().map(|row| row.get("status")),
        session_fingerprint: session.and_then(|row| row.get("hardware_fingerprint")),
        active_public_key,
    })
}

fn verify_security_posture(
    signed: &SignedSecurityPosture,
    computed_hash: &str,
    context: &SecurityContext,
    policy: &SecurityPolicy,
) -> SecurityPostureVerification {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let posture_hash_valid = computed_hash == signed.posture_hash;
    if !posture_hash_valid {
        errors.push("posture_hash does not match canonical payload".to_string());
    }
    let session_bound = context
        .session_status
        .as_deref()
        .is_some_and(|status| matches!(status, "online" | "degraded"));
    if !session_bound {
        errors.push("security posture requires an online or degraded remote session".to_string());
    }
    let fingerprint_bound = context.session_fingerprint.as_deref()
        == Some(signed.payload.hardware_fingerprint.as_str());
    if !fingerprint_bound {
        errors.push(
            "security posture hardware fingerprint does not match the remote session".to_string(),
        );
    }
    let active_key_bound = context.active_public_key.is_some();
    if !active_key_bound {
        errors.push("security posture public_key_id is not active for this device".to_string());
    }
    let signature_message =
        security_posture_signature_message(&signed.payload, computed_hash, &signed.public_key_id)
            .unwrap_or_default();
    let signature_valid = context
        .active_public_key
        .as_ref()
        .is_some_and(|public_key| {
            verify_message(public_key, signature_message.as_bytes(), &signed.signature)
                .unwrap_or(false)
        });
    if !signature_valid {
        errors.push("security posture signature is invalid".to_string());
    }

    let release_policy_satisfied = release_policy_satisfied(signed, policy, &mut warnings);
    let key_storage_satisfied = key_storage_satisfied(signed, policy, &mut warnings);
    let attestation_satisfied = attestation_satisfied(signed, policy, &mut warnings);
    let artifact_integrity_satisfied = artifact_integrity_satisfied(signed, policy, &mut warnings);
    if signed.payload.key_storage.storage_backend == "software_file" {
        warnings.push("provider private key is still backed by software file storage".to_string());
    }

    SecurityPostureVerification {
        schema_version: SECURITY_POSTURE_SCHEMA_VERSION.to_string(),
        posture_hash_valid,
        signature_valid,
        session_bound,
        fingerprint_bound,
        active_key_bound,
        release_policy_satisfied,
        key_storage_satisfied,
        attestation_satisfied,
        artifact_integrity_satisfied,
        warnings,
        errors,
    }
}

fn release_policy_satisfied(
    signed: &SignedSecurityPosture,
    policy: &SecurityPolicy,
    warnings: &mut Vec<String>,
) -> bool {
    let release = &signed.payload.release;
    let mut satisfied = true;
    if !policy
        .accepted_release_channels
        .iter()
        .any(|channel| channel == &release.release_channel)
    {
        warnings.push("agent release channel is not accepted by policy".to_string());
        satisfied = false;
    }
    if policy.require_signed_agent_release
        && (!release.signature_verified
            || release.signer_key_id.as_deref().unwrap_or("").is_empty())
    {
        warnings.push("agent release signature is required but not verified".to_string());
        satisfied = false;
    }
    if let Some(minimum) = &policy.min_agent_version
        && !version_at_least(&signed.payload.agent_version, minimum)
    {
        warnings.push("agent version is below the configured minimum".to_string());
        satisfied = false;
    }
    satisfied
}

fn key_storage_satisfied(
    signed: &SignedSecurityPosture,
    policy: &SecurityPolicy,
    warnings: &mut Vec<String>,
) -> bool {
    if !policy.require_hardware_backed_key {
        return true;
    }
    if signed.payload.key_storage.hardware_backed
        && !signed.payload.key_storage.private_key_exportable
    {
        return true;
    }
    warnings.push("hardware-backed non-exportable provider key storage is required".to_string());
    false
}

fn attestation_satisfied(
    signed: &SignedSecurityPosture,
    policy: &SecurityPolicy,
    warnings: &mut Vec<String>,
) -> bool {
    let attestation = &signed.payload.attestation;
    let mode_accepted = policy
        .accepted_attestation_modes
        .iter()
        .any(|mode| mode == &attestation.mode);
    if !mode_accepted {
        warnings.push("attestation mode is not accepted by policy".to_string());
        return false;
    }
    if !policy.require_remote_attestation {
        return true;
    }
    if attestation.mode != "none"
        && attestation
            .evidence_hash
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        && attestation.quote_verified_locally
    {
        return true;
    }
    warnings.push("remote attestation evidence is required but incomplete".to_string());
    false
}

fn artifact_integrity_satisfied(
    signed: &SignedSecurityPosture,
    policy: &SecurityPolicy,
    warnings: &mut Vec<String>,
) -> bool {
    let artifact = &signed.payload.artifact_integrity;
    let mut satisfied = true;
    if policy.require_sbom_hash
        && artifact
            .sbom_hash
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        warnings.push("SBOM hash is required but missing".to_string());
        satisfied = false;
    }
    if artifact.vulnerability_scan_status == "failed" || artifact.dependency_scan_status == "failed"
    {
        warnings.push("artifact or dependency scan failed".to_string());
        satisfied = false;
    }
    satisfied
}

fn posture_status(verification: &SecurityPostureVerification) -> String {
    if verification.release_policy_satisfied
        && verification.key_storage_satisfied
        && verification.attestation_satisfied
        && verification.artifact_integrity_satisfied
    {
        "verified".to_string()
    } else {
        "needs_hardening".to_string()
    }
}

async fn record_rejected_security_posture(
    transaction: &Transaction<'_>,
    request_id: &str,
    authorized: &AuthorizedSession,
    posture_hash: &str,
    signed: &SignedSecurityPosture,
    verification: &SecurityPostureVerification,
) -> Result<(), SessionError> {
    let metadata = serde_json::json!({
        "posture_hash": posture_hash,
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
            entity_type: "device_security_posture",
            entity_id: &authorized.device_id,
            event_type: "security_posture.rejected",
            idempotency_key: None,
            summary: "signed security posture rejected",
            metadata_json: &metadata,
        },
    )
    .await?;
    Ok(())
}

fn validate_signed_posture_shape(
    signed: &SignedSecurityPosture,
    authorized: &AuthorizedSession,
) -> Result<(), SessionError> {
    let payload = &signed.payload;
    if payload.schema_version != SECURITY_POSTURE_SCHEMA_VERSION
        || signed.canonicalization_version != SECURITY_POSTURE_CANONICALIZATION_VERSION
    {
        return Err(SessionError::Invalid(
            "unsupported security posture schema or canonicalization version".to_string(),
        ));
    }
    if payload.provider_id != authorized.provider_id
        || payload.device_id != authorized.device_id
        || payload.session_id != authorized.session_id
    {
        return Err(SessionError::Unauthorized);
    }
    for (field, value) in [
        ("agent_version", payload.agent_version.as_str()),
        (
            "hardware_fingerprint",
            payload.hardware_fingerprint.as_str(),
        ),
        ("observed_at", payload.observed_at.as_str()),
        ("os", payload.os.as_str()),
        ("architecture", payload.architecture.as_str()),
        ("release_channel", payload.release.release_channel.as_str()),
        (
            "key_storage_backend",
            payload.key_storage.storage_backend.as_str(),
        ),
        ("attestation_mode", payload.attestation.mode.as_str()),
        (
            "dependency_scan_status",
            payload.artifact_integrity.dependency_scan_status.as_str(),
        ),
        (
            "vulnerability_scan_status",
            payload
                .artifact_integrity
                .vulnerability_scan_status
                .as_str(),
        ),
        (
            "secrets_backend",
            payload.hardening.secrets_backend.as_str(),
        ),
    ] {
        validate_short_ascii(field, value)?;
    }
    for value in [
        payload.release.binary_hash.as_deref(),
        payload.release.signer_key_id.as_deref(),
        payload.attestation.evidence_hash.as_deref(),
        payload.artifact_integrity.sbom_hash.as_deref(),
        payload.hardening.sandbox_runtime.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_short_ascii("optional security posture field", value)?;
    }
    if signed.public_key_id.trim().is_empty()
        || signed.signature.trim().is_empty()
        || signed.posture_hash.trim().is_empty()
    {
        return Err(SessionError::Invalid(
            "security posture signature fields are required".to_string(),
        ));
    }
    if !matches!(
        payload.artifact_integrity.dependency_scan_status.as_str(),
        "not_run" | "passed" | "failed" | "unknown"
    ) || !matches!(
        payload
            .artifact_integrity
            .vulnerability_scan_status
            .as_str(),
        "not_run" | "passed" | "failed" | "unknown"
    ) {
        return Err(SessionError::Invalid(
            "security scan status must be not_run, passed, failed, or unknown".to_string(),
        ));
    }
    if payload
        .warnings
        .iter()
        .any(|warning| warning.len() > 256 || contains_secret_hint(warning))
    {
        return Err(SessionError::Invalid(
            "security posture warnings must be redacted".to_string(),
        ));
    }
    Ok(())
}

fn validate_short_ascii(field: &str, value: &str) -> Result<(), SessionError> {
    if value.trim().is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii() && !character.is_ascii_control())
    {
        return Err(SessionError::Invalid(format!(
            "security posture {field} is invalid or unredacted"
        )));
    }
    Ok(())
}

fn contains_secret_hint(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    ["secret", "token", "password", "private_key", "pix_key"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn version_at_least(actual: &str, minimum: &str) -> bool {
    let actual_parts = version_parts(actual);
    let minimum_parts = version_parts(minimum);
    for index in 0..actual_parts.len().max(minimum_parts.len()) {
        let actual = *actual_parts.get(index).unwrap_or(&0);
        let minimum = *minimum_parts.get(index).unwrap_or(&0);
        if actual > minimum {
            return true;
        }
        if actual < minimum {
            return false;
        }
    }
    true
}

fn version_parts(value: &str) -> Vec<u64> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn security_posture_select_columns() -> &'static str {
    "SELECT posture_id, provider_id, device_id, session_id, schema_version, policy_version, status, posture_hash, public_key_id, agent_version, release_channel, key_storage_backend, key_hardware_backed, private_key_exportable, attestation_mode, attestation_evidence_hash, binary_hash, sbom_hash, vulnerability_scan_status, dependency_scan_status, hardware_fingerprint, observed_at, server_received_at, verification_json FROM device_security_postures"
}

fn security_posture_from_row(row: Row) -> Result<SecurityPostureRecord, SessionError> {
    let verification_json: String = row.get("verification_json");
    let verification = serde_json::from_str::<SecurityPostureVerification>(&verification_json)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    Ok(SecurityPostureRecord {
        posture_id: row.get("posture_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        schema_version: row.get("schema_version"),
        policy_version: row.get("policy_version"),
        status: row.get("status"),
        posture_hash: row.get("posture_hash"),
        public_key_id: row.get("public_key_id"),
        agent_version: row.get("agent_version"),
        release_channel: row.get("release_channel"),
        key_storage_backend: row.get("key_storage_backend"),
        key_hardware_backed: row.get("key_hardware_backed"),
        private_key_exportable: row.get("private_key_exportable"),
        attestation_mode: row.get("attestation_mode"),
        attestation_evidence_hash: row.get("attestation_evidence_hash"),
        binary_hash: row.get("binary_hash"),
        sbom_hash: row.get("sbom_hash"),
        vulnerability_scan_status: row.get("vulnerability_scan_status"),
        dependency_scan_status: row.get("dependency_scan_status"),
        hardware_fingerprint: row.get("hardware_fingerprint"),
        observed_at: row.get("observed_at"),
        server_received_at: row.get("server_received_at"),
        verification,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{
        AgentReleasePosture, ArtifactIntegrityPosture, AttestationPosture, KeyStoragePosture,
        SECURITY_POSTURE_CANONICALIZATION_VERSION, SECURITY_POSTURE_SCHEMA_VERSION,
        SecurityHardeningPosture, SecurityPosturePayload, security_posture_hash,
    };

    fn signed_posture() -> SignedSecurityPosture {
        let payload = SecurityPosturePayload {
            schema_version: SECURITY_POSTURE_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            agent_version: "0.2.0".to_string(),
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            observed_at: "2026-07-14T00:00:00Z".to_string(),
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
            key_storage: KeyStoragePosture {
                storage_backend: "tpm".to_string(),
                hardware_backed: true,
                private_key_exportable: false,
                encrypted_at_rest: true,
            },
            release: AgentReleasePosture {
                release_channel: "stable".to_string(),
                binary_hash: Some("sha256:binary".to_string()),
                signature_verified: true,
                signer_key_id: Some("release_key_1".to_string()),
                auto_update_enabled: true,
            },
            attestation: AttestationPosture {
                mode: "tpm".to_string(),
                evidence_hash: Some("sha256:quote".to_string()),
                quote_verified_locally: true,
            },
            artifact_integrity: ArtifactIntegrityPosture {
                sbom_hash: Some("sha256:sbom".to_string()),
                dependency_scan_status: "passed".to_string(),
                vulnerability_scan_status: "passed".to_string(),
            },
            hardening: SecurityHardeningPosture {
                secrets_backend: "os_keychain".to_string(),
                sandbox_runtime: Some("docker".to_string()),
                rbac_enforced: true,
                admin_approval_required: true,
            },
            warnings: vec![],
        };
        let posture_hash = security_posture_hash(&payload).unwrap();
        SignedSecurityPosture {
            payload,
            posture_hash,
            public_key_id: "key_1".to_string(),
            signature: "signature".to_string(),
            canonicalization_version: SECURITY_POSTURE_CANONICALIZATION_VERSION.to_string(),
        }
    }

    #[test]
    fn policy_marks_strong_posture_satisfied() {
        let signed = signed_posture();
        let policy = SecurityPolicy {
            min_agent_version: Some("0.1.0".to_string()),
            require_signed_agent_release: true,
            require_hardware_backed_key: true,
            require_remote_attestation: true,
            require_sbom_hash: true,
            accepted_release_channels: vec!["stable".to_string()],
            accepted_attestation_modes: vec!["tpm".to_string()],
        };
        let mut warnings = Vec::new();
        assert!(release_policy_satisfied(&signed, &policy, &mut warnings));
        assert!(key_storage_satisfied(&signed, &policy, &mut warnings));
        assert!(attestation_satisfied(&signed, &policy, &mut warnings));
        assert!(artifact_integrity_satisfied(
            &signed,
            &policy,
            &mut warnings
        ));
        assert!(warnings.is_empty());
    }

    #[test]
    fn version_compare_handles_dotted_versions() {
        assert!(version_at_least("1.10.0", "1.9.9"));
        assert!(version_at_least("1.0", "1.0.0"));
        assert!(!version_at_least("0.9.9", "1.0.0"));
    }
}
