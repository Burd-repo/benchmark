use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::{AuthorizedSession, SessionError};
use burd_protocol::{
    BENCHMARK_PROFILE_SCHEMA_VERSION, BENCHMARK_RESULT_CANONICALIZATION_VERSION,
    BENCHMARK_RESULT_SCHEMA_VERSION, BenchmarkProfileRecord, BenchmarkProfileThresholds,
    BenchmarkResultMetrics, BenchmarkResultRecord, BenchmarkResultVerification,
    ListBenchmarkProfilesResponse, ListProviderBenchmarkResultsResponse, SignedBenchmarkResult,
    SubmitBenchmarkResultResponse, UpsertBenchmarkProfileRequest, UpsertBenchmarkProfileResponse,
    benchmark_result_hash, benchmark_result_signature_message, verify_message,
};
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

impl Database {
    pub async fn upsert_benchmark_profile(
        &self,
        request_id: &str,
        request: &UpsertBenchmarkProfileRequest,
    ) -> Result<UpsertBenchmarkProfileResponse, SessionError> {
        validate_profile_request(request)?;
        let parameters_json = serde_json::to_string(&normalized_json_object(
            &request.parameters,
            "benchmark profile parameters",
        )?)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let thresholds_json = serde_json::to_string(&request.thresholds)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let status = request.status.as_deref().unwrap_or("active").to_string();
        let now = Utc::now().to_rfc3339();

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .execute(
                "INSERT INTO benchmark_profiles (profile_id, profile_version, schema_version, workload_type, display_name, description, image_digest, model_hash, artifact_hash, required_backend, min_vram_gb, parameters_json, warmup_seconds, duration_seconds, sample_count, thresholds_json, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $18) ON CONFLICT (profile_id, profile_version) DO UPDATE SET workload_type = EXCLUDED.workload_type, display_name = EXCLUDED.display_name, description = EXCLUDED.description, image_digest = EXCLUDED.image_digest, model_hash = EXCLUDED.model_hash, artifact_hash = EXCLUDED.artifact_hash, required_backend = EXCLUDED.required_backend, min_vram_gb = EXCLUDED.min_vram_gb, parameters_json = EXCLUDED.parameters_json, warmup_seconds = EXCLUDED.warmup_seconds, duration_seconds = EXCLUDED.duration_seconds, sample_count = EXCLUDED.sample_count, thresholds_json = EXCLUDED.thresholds_json, status = EXCLUDED.status, updated_at = EXCLUDED.updated_at",
                &[
                    &request.profile_id,
                    &request.profile_version,
                    &BENCHMARK_PROFILE_SCHEMA_VERSION,
                    &request.workload_type,
                    &request.display_name,
                    &request.description,
                    &request.image_digest,
                    &request.model_hash,
                    &request.artifact_hash,
                    &request.required_backend,
                    &request.min_vram_gb,
                    &parameters_json,
                    &(request.warmup_seconds as i32),
                    &(request.duration_seconds as i32),
                    &(request.sample_count as i32),
                    &thresholds_json,
                    &status,
                    &now,
                ],
            )
            .await?;
        let row = transaction
            .query_one(
                &format!(
                    "{} WHERE profile_id = $1 AND profile_version = $2",
                    profile_select_columns()
                ),
                &[&request.profile_id, &request.profile_version],
            )
            .await?;
        let profile = profile_from_row(row)?;
        let audit_metadata = serde_json::json!({
            "profile_id": profile.profile_id,
            "profile_version": profile.profile_version,
            "workload_type": profile.workload_type,
            "image_digest": profile.image_digest,
            "status": profile.status,
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "benchmark_profile",
                entity_id: &profile.profile_id,
                event_type: "benchmark_profile.upserted",
                idempotency_key: None,
                summary: "benchmark profile v2 upserted",
                metadata_json: &audit_metadata,
            },
        )
        .await?;
        transaction.commit().await?;

        Ok(UpsertBenchmarkProfileResponse {
            request_id: request_id.to_string(),
            profile,
        })
    }

    pub async fn list_benchmark_profiles(
        &self,
        request_id: &str,
    ) -> Result<ListBenchmarkProfilesResponse, SessionError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} ORDER BY workload_type, profile_id, profile_version DESC",
                    profile_select_columns()
                ),
                &[],
            )
            .await?;
        let profiles = rows
            .into_iter()
            .map(profile_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListBenchmarkProfilesResponse {
            request_id: request_id.to_string(),
            profiles,
        })
    }

    pub async fn submit_benchmark_result(
        &self,
        request_id: &str,
        authorized: &AuthorizedSession,
        signed: &SignedBenchmarkResult,
    ) -> Result<SubmitBenchmarkResultResponse, SessionError> {
        validate_signed_result_shape(signed, authorized)?;
        let computed_hash =
            benchmark_result_hash(&signed.payload).map_err(SessionError::Invalid)?;
        if computed_hash != signed.result_hash {
            return Err(SessionError::Invalid(
                "benchmark result_hash does not match canonical payload".to_string(),
            ));
        }

        let parameters_json = serde_json::to_string(&normalized_json_object(
            &signed.payload.parameters,
            "benchmark result parameters",
        )?)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let metrics_json = serde_json::to_string(&signed.payload.metrics)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let warnings_json = serde_json::to_string(&signed.payload.warnings)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let server_received_at = Utc::now().to_rfc3339();
        let result_id = format!("benchmark_result_{}", Uuid::new_v4());

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let session_fingerprint =
            assert_session_accepts_benchmark_result(&transaction, authorized, signed).await?;
        let profile = benchmark_profile_for_result(
            &transaction,
            &signed.payload.profile_id,
            &signed.payload.profile_version,
        )
        .await?;
        let active_public_key = transaction
            .query_opt(
                "SELECT public_key FROM provider_public_keys WHERE public_key_id = $1 AND provider_id = $2 AND device_id = $3 AND status = 'active'",
                &[&signed.public_key_id, &authorized.provider_id, &authorized.device_id],
            )
            .await?
            .map(|row| row.get::<_, String>("public_key"));
        let verification = verify_benchmark_result(
            signed,
            &profile,
            &computed_hash,
            authorized,
            &session_fingerprint,
            active_public_key.as_deref(),
        )?;
        if !verification.result_hash_valid
            || !verification.signature_valid
            || !verification.session_bound
            || !verification.profile_bound
            || !verification.backend_bound
            || !verification.fingerprint_bound
            || !verification.image_bound
            || !verification.model_bound
            || !verification.artifact_bound
            || !verification.profile_configuration_bound
        {
            return Err(SessionError::SignatureInvalid);
        }
        let status = if verification.metrics_satisfied {
            "succeeded"
        } else {
            "failed"
        };
        let verification_json = serde_json::to_string(&verification)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;

        if let Some(existing) = transaction
            .query_opt(
                "SELECT result_hash FROM benchmark_results WHERE provider_id = $1 AND device_id = $2 AND run_id = $3 FOR UPDATE",
                &[&authorized.provider_id, &authorized.device_id, &signed.payload.run_id],
            )
            .await?
        {
            let existing_hash: String = existing.get("result_hash");
            if existing_hash != computed_hash {
                return Err(SessionError::Conflict(
                    "benchmark run_id already exists with a different result_hash".to_string(),
                ));
            }
        }

        let inserted = transaction
            .execute(
                "INSERT INTO benchmark_results (result_id, provider_id, device_id, session_id, run_id, profile_id, profile_version, schema_version, workload_type, backend, hardware_fingerprint, gpu_uuid, image_digest, model_hash, artifact_hash, parameters_json, warmup_seconds, duration_seconds, sample_count, started_at, completed_at, server_received_at, driver_version, cuda_driver_version, cuda_runtime_version, metrics_json, telemetry_window_hash, result_hash, public_key_id, signature, canonicalization_version, status, verification_json, warnings_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34) ON CONFLICT (result_hash) DO NOTHING",
                &[
                    &result_id,
                    &authorized.provider_id,
                    &authorized.device_id,
                    &authorized.session_id,
                    &signed.payload.run_id,
                    &signed.payload.profile_id,
                    &signed.payload.profile_version,
                    &BENCHMARK_RESULT_SCHEMA_VERSION,
                    &signed.payload.workload_type,
                    &signed.payload.backend,
                    &signed.payload.hardware_fingerprint,
                    &signed.payload.gpu_uuid,
                    &signed.payload.image_digest,
                    &signed.payload.model_hash,
                    &signed.payload.artifact_hash,
                    &parameters_json,
                    &(signed.payload.warmup_seconds as i32),
                    &(signed.payload.duration_seconds as i32),
                    &(signed.payload.sample_count as i32),
                    &signed.payload.started_at,
                    &signed.payload.completed_at,
                    &server_received_at,
                    &signed.payload.driver_version,
                    &signed.payload.cuda_driver_version,
                    &signed.payload.cuda_runtime_version,
                    &metrics_json,
                    &signed.payload.telemetry_window_hash,
                    &computed_hash,
                    &signed.public_key_id,
                    &signed.signature,
                    &BENCHMARK_RESULT_CANONICALIZATION_VERSION,
                    &status,
                    &verification_json,
                    &warnings_json,
                ],
            )
            .await?
            == 1;
        let row = transaction
            .query_one(
                &format!("{} WHERE result_hash = $1", result_select_columns()),
                &[&computed_hash],
            )
            .await?;
        let result = result_from_row(row)?;
        if inserted {
            let audit_metadata = serde_json::json!({
                "result_id": result.result_id,
                "profile_id": result.profile_id,
                "profile_version": result.profile_version,
                "run_id": result.run_id,
                "status": result.status,
                "metrics_satisfied": result.verification.metrics_satisfied,
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "device_key",
                    actor_id: Some(signed.public_key_id.clone()),
                    entity_type: "benchmark_result",
                    entity_id: &result.result_id,
                    event_type: "benchmark_result.accepted",
                    idempotency_key: None,
                    summary: "signed benchmark result accepted",
                    metadata_json: &audit_metadata,
                },
            )
            .await?;
        }
        transaction.commit().await?;

        Ok(SubmitBenchmarkResultResponse {
            request_id: request_id.to_string(),
            duplicate: !inserted,
            result,
        })
    }

    pub async fn list_provider_benchmark_results(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListProviderBenchmarkResultsResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let limit = limit.clamp(1, 200) as i64;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE provider_id = $1 ORDER BY completed_at DESC, server_received_at DESC LIMIT $2",
                    result_select_columns()
                ),
                &[&provider_id, &limit],
            )
            .await?;
        let results = rows
            .into_iter()
            .map(result_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListProviderBenchmarkResultsResponse {
            request_id: request_id.to_string(),
            results,
        })
    }
}

async fn assert_session_accepts_benchmark_result(
    transaction: &Transaction<'_>,
    authorized: &AuthorizedSession,
    signed: &SignedBenchmarkResult,
) -> Result<String, SessionError> {
    let row = transaction
        .query_opt(
            "SELECT s.status, s.hardware_fingerprint, p.status AS provider_status, d.status AS device_status FROM provider_sessions s JOIN providers p ON p.provider_id = s.provider_id JOIN devices d ON d.device_id = s.device_id WHERE s.session_id = $1 AND s.provider_id = $2 AND s.device_id = $3 AND d.provider_id = $2 FOR UPDATE",
            &[&authorized.session_id, &authorized.provider_id, &authorized.device_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
    let provider_status: String = row.get("provider_status");
    let device_status: String = row.get("device_status");
    if matches!(provider_status.as_str(), "blocked" | "quarantined") || device_status != "active" {
        return Err(SessionError::Revoked);
    }
    let session_status: String = row.get("status");
    if !matches!(session_status.as_str(), "online" | "degraded") {
        return Err(SessionError::Conflict(
            "benchmark result requires an online or degraded remote session".to_string(),
        ));
    }
    let fingerprint = row
        .get::<_, Option<String>>("hardware_fingerprint")
        .ok_or_else(|| SessionError::Conflict("remote session fingerprint missing".to_string()))?;
    if fingerprint != signed.payload.hardware_fingerprint {
        return Err(SessionError::Conflict(
            "benchmark result hardware fingerprint does not match the remote session".to_string(),
        ));
    }
    Ok(fingerprint)
}

async fn benchmark_profile_for_result(
    transaction: &Transaction<'_>,
    profile_id: &str,
    profile_version: &str,
) -> Result<BenchmarkProfileRecord, SessionError> {
    let row = transaction
        .query_opt(
            &format!(
                "{} WHERE profile_id = $1 AND profile_version = $2 AND status = 'active'",
                profile_select_columns()
            ),
            &[&profile_id, &profile_version],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("benchmark profile not found".to_string()))?;
    profile_from_row(row)
}

fn verify_benchmark_result(
    signed: &SignedBenchmarkResult,
    profile: &BenchmarkProfileRecord,
    computed_hash: &str,
    authorized: &AuthorizedSession,
    session_fingerprint: &str,
    active_public_key: Option<&str>,
) -> Result<BenchmarkResultVerification, SessionError> {
    let payload = &signed.payload;
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let signature_message =
        benchmark_result_signature_message(payload, computed_hash, &signed.public_key_id)
            .map_err(SessionError::Invalid)?;
    let signature_valid = active_public_key.is_some_and(|public_key| {
        verify_message(public_key, signature_message.as_bytes(), &signed.signature).unwrap_or(false)
    });
    if active_public_key.is_none() {
        errors.push("benchmark result public_key_id is not active for this device".to_string());
    } else if !signature_valid {
        errors.push("benchmark result Ed25519 signature is invalid".to_string());
    }
    let profile_bound = payload.profile_id == profile.profile_id
        && payload.profile_version == profile.profile_version
        && payload.workload_type == profile.workload_type;
    if !profile_bound {
        errors.push("benchmark result profile binding does not match registry".to_string());
    }
    let backend_bound = payload.backend == profile.required_backend;
    if !backend_bound {
        errors.push("benchmark result backend does not match profile".to_string());
    }
    let image_bound = payload.image_digest == profile.image_digest;
    if !image_bound {
        errors.push("benchmark result image digest does not match profile".to_string());
    }
    let model_bound = profile
        .model_hash
        .as_ref()
        .map(|model_hash| payload.model_hash.as_ref() == Some(model_hash))
        .unwrap_or(true);
    if !model_bound {
        errors.push("benchmark result model hash does not match profile".to_string());
    }
    let artifact_bound = profile
        .artifact_hash
        .as_ref()
        .map(|artifact_hash| payload.artifact_hash.as_ref() == Some(artifact_hash))
        .unwrap_or(true);
    if !artifact_bound {
        errors.push("benchmark result artifact hash does not match profile".to_string());
    }
    let payload_parameters =
        normalized_json_object(&payload.parameters, "benchmark result parameters")?;
    let profile_parameters =
        normalized_json_object(&profile.parameters, "benchmark profile parameters")?;
    let profile_configuration_bound = payload_parameters == profile_parameters
        && payload.warmup_seconds == profile.warmup_seconds
        && payload.duration_seconds == profile.duration_seconds
        && payload.sample_count == profile.sample_count;
    if !profile_configuration_bound {
        errors.push("benchmark result configuration does not match profile".to_string());
    }
    let metrics_satisfied =
        metrics_satisfy_thresholds(&payload.metrics, &profile.thresholds, &mut warnings);

    Ok(BenchmarkResultVerification {
        schema_version: BENCHMARK_RESULT_SCHEMA_VERSION.to_string(),
        result_hash_valid: signed.result_hash == computed_hash,
        signature_valid,
        session_bound: payload.session_id == authorized.session_id
            && payload.provider_id == authorized.provider_id
            && payload.device_id == authorized.device_id,
        profile_bound,
        backend_bound,
        fingerprint_bound: payload.hardware_fingerprint == session_fingerprint,
        image_bound,
        model_bound,
        artifact_bound,
        profile_configuration_bound,
        metrics_satisfied,
        warnings,
        errors,
    })
}

fn metrics_satisfy_thresholds(
    metrics: &BenchmarkResultMetrics,
    thresholds: &BenchmarkProfileThresholds,
    warnings: &mut Vec<String>,
) -> bool {
    let mut passed = true;
    for (label, measured, minimum) in [
        (
            "tokens_per_second",
            metrics.tokens_per_second,
            thresholds.min_tokens_per_second,
        ),
        (
            "sustained_tokens_per_second",
            metrics.sustained_tokens_per_second,
            thresholds.min_sustained_tokens_per_second,
        ),
        (
            "requests_per_second",
            metrics.requests_per_second,
            thresholds.min_requests_per_second,
        ),
    ] {
        if let Some(minimum) = minimum
            && measured.is_none_or(|value| value < minimum)
        {
            passed = false;
            warnings.push(format!("{label}_below_profile_threshold"));
        }
    }
    for (label, measured, maximum) in [
        ("ttft_ms", metrics.ttft_ms, thresholds.max_ttft_ms),
        (
            "latency_p95_ms",
            metrics.latency_p95_ms,
            thresholds.max_latency_p95_ms,
        ),
        (
            "error_rate_percent",
            metrics.error_rate_percent,
            thresholds.max_error_rate_percent,
        ),
    ] {
        if let Some(maximum) = maximum
            && measured.is_none_or(|value| value > maximum)
        {
            passed = false;
            warnings.push(format!("{label}_above_profile_threshold"));
        }
    }
    passed
}

fn validate_profile_request(request: &UpsertBenchmarkProfileRequest) -> Result<(), SessionError> {
    validate_id("profile_id", &request.profile_id, 128)?;
    validate_id("profile_version", &request.profile_version, 64)?;
    validate_id("workload_type", &request.workload_type, 64)?;
    validate_id("required_backend", &request.required_backend, 32)?;
    validate_digest("image_digest", &request.image_digest)?;
    if request
        .model_hash
        .as_deref()
        .is_some_and(|value| validate_digest("model_hash", value).is_err())
    {
        return Err(SessionError::Invalid("model_hash is invalid".to_string()));
    }
    if request
        .artifact_hash
        .as_deref()
        .is_some_and(|value| validate_digest("artifact_hash", value).is_err())
    {
        return Err(SessionError::Invalid(
            "artifact_hash is invalid".to_string(),
        ));
    }
    if request.display_name.trim().is_empty() || request.display_name.len() > 160 {
        return Err(SessionError::Invalid("display_name is invalid".to_string()));
    }
    if request
        .description
        .as_deref()
        .is_some_and(|value| value.len() > 1000 || contains_secret_text(value))
    {
        return Err(SessionError::Invalid("description is invalid".to_string()));
    }
    if !request.min_vram_gb.is_finite() || request.min_vram_gb < 0.0 || request.min_vram_gb > 1024.0
    {
        return Err(SessionError::Invalid(
            "min_vram_gb must be finite and between 0 and 1024".to_string(),
        ));
    }
    if request.warmup_seconds > 3600
        || request.duration_seconds == 0
        || request.duration_seconds > 24 * 3600
        || request.sample_count == 0
        || request.sample_count > 1_000_000
    {
        return Err(SessionError::Invalid(
            "benchmark profile duration, warmup, or sample_count is invalid".to_string(),
        ));
    }
    validate_thresholds(&request.thresholds)?;
    validate_status(request.status.as_deref().unwrap_or("active"))?;
    validate_json_object(&request.parameters, "benchmark profile parameters")
}

fn validate_signed_result_shape(
    signed: &SignedBenchmarkResult,
    authorized: &AuthorizedSession,
) -> Result<(), SessionError> {
    let payload = &signed.payload;
    if signed.canonicalization_version != BENCHMARK_RESULT_CANONICALIZATION_VERSION
        || payload.schema_version != BENCHMARK_RESULT_SCHEMA_VERSION
    {
        return Err(SessionError::Invalid(
            "unsupported benchmark result schema or canonicalization version".to_string(),
        ));
    }
    if payload.provider_id != authorized.provider_id
        || payload.device_id != authorized.device_id
        || payload.session_id != authorized.session_id
    {
        return Err(SessionError::Unauthorized);
    }
    for (label, value, max_len) in [
        ("provider_id", payload.provider_id.as_str(), 128),
        ("device_id", payload.device_id.as_str(), 128),
        ("session_id", payload.session_id.as_str(), 128),
        ("run_id", payload.run_id.as_str(), 128),
        ("profile_id", payload.profile_id.as_str(), 128),
        ("profile_version", payload.profile_version.as_str(), 64),
        ("workload_type", payload.workload_type.as_str(), 64),
        ("backend", payload.backend.as_str(), 32),
        (
            "hardware_fingerprint",
            payload.hardware_fingerprint.as_str(),
            160,
        ),
        ("gpu_uuid", payload.gpu_uuid.as_str(), 128),
        ("driver_version", payload.driver_version.as_str(), 96),
        ("result_hash", signed.result_hash.as_str(), 128),
        ("public_key_id", signed.public_key_id.as_str(), 128),
    ] {
        if !is_bounded_ascii(value, max_len) {
            return Err(SessionError::Invalid(format!("{label} is invalid")));
        }
    }
    validate_digest("image_digest", &payload.image_digest)?;
    if payload
        .model_hash
        .as_deref()
        .is_some_and(|value| validate_digest("model_hash", value).is_err())
    {
        return Err(SessionError::Invalid("model_hash is invalid".to_string()));
    }
    if payload
        .artifact_hash
        .as_deref()
        .is_some_and(|value| validate_digest("artifact_hash", value).is_err())
    {
        return Err(SessionError::Invalid(
            "artifact_hash is invalid".to_string(),
        ));
    }
    if payload.warmup_seconds > 3600
        || payload.duration_seconds == 0
        || payload.duration_seconds > 24 * 3600
        || payload.sample_count == 0
        || payload.sample_count > 1_000_000
    {
        return Err(SessionError::Invalid(
            "benchmark result duration, warmup, or sample_count is invalid".to_string(),
        ));
    }
    let started_at = parse_rfc3339("started_at", &payload.started_at)?;
    let completed_at = parse_rfc3339("completed_at", &payload.completed_at)?;
    if completed_at < started_at {
        return Err(SessionError::Invalid(
            "completed_at must be after started_at".to_string(),
        ));
    }
    if completed_at.with_timezone(&Utc) > Utc::now() + Duration::minutes(5) {
        return Err(SessionError::Invalid(
            "completed_at is too far in the future".to_string(),
        ));
    }
    validate_metrics(&payload.metrics)?;
    validate_json_object(&payload.parameters, "benchmark result parameters")?;
    if payload.warnings.len() > 64
        || payload
            .warnings
            .iter()
            .any(|value| value.len() > 256 || contains_secret_text(value))
    {
        return Err(SessionError::Invalid(
            "benchmark result warnings must be small and redacted".to_string(),
        ));
    }
    Ok(())
}

fn validate_thresholds(thresholds: &BenchmarkProfileThresholds) -> Result<(), SessionError> {
    for (label, value, maximum) in [
        (
            "min_tokens_per_second",
            thresholds.min_tokens_per_second,
            1_000_000.0,
        ),
        (
            "min_sustained_tokens_per_second",
            thresholds.min_sustained_tokens_per_second,
            1_000_000.0,
        ),
        (
            "min_requests_per_second",
            thresholds.min_requests_per_second,
            1_000_000.0,
        ),
        (
            "max_ttft_ms",
            thresholds.max_ttft_ms,
            24.0 * 3600.0 * 1000.0,
        ),
        (
            "max_latency_p95_ms",
            thresholds.max_latency_p95_ms,
            24.0 * 3600.0 * 1000.0,
        ),
        (
            "max_error_rate_percent",
            thresholds.max_error_rate_percent,
            100.0,
        ),
    ] {
        if let Some(value) = value {
            validate_finite_range(label, value, 0.0, maximum)?;
        }
    }
    Ok(())
}

fn validate_metrics(metrics: &BenchmarkResultMetrics) -> Result<(), SessionError> {
    let mut has_metric = false;
    for (label, value, maximum) in [
        ("tokens_per_second", metrics.tokens_per_second, 1_000_000.0),
        (
            "sustained_tokens_per_second",
            metrics.sustained_tokens_per_second,
            1_000_000.0,
        ),
        (
            "requests_per_second",
            metrics.requests_per_second,
            1_000_000.0,
        ),
        ("ttft_ms", metrics.ttft_ms, 24.0 * 3600.0 * 1000.0),
        (
            "latency_p50_ms",
            metrics.latency_p50_ms,
            24.0 * 3600.0 * 1000.0,
        ),
        (
            "latency_p95_ms",
            metrics.latency_p95_ms,
            24.0 * 3600.0 * 1000.0,
        ),
        (
            "latency_p99_ms",
            metrics.latency_p99_ms,
            24.0 * 3600.0 * 1000.0,
        ),
        (
            "performance_per_watt",
            metrics.performance_per_watt,
            1_000_000.0,
        ),
        ("energy_joules", metrics.energy_joules, 1_000_000_000.0),
        (
            "vram_pressure_percent",
            metrics.vram_pressure_percent,
            100.0,
        ),
        (
            "gpu_utilization_percent",
            metrics.gpu_utilization_percent,
            100.0,
        ),
        (
            "memory_utilization_percent",
            metrics.memory_utilization_percent,
            100.0,
        ),
        ("temperature_c", metrics.temperature_c, 150.0),
        ("power_watts", metrics.power_watts, 10_000.0),
        ("error_rate_percent", metrics.error_rate_percent, 100.0),
    ] {
        if let Some(value) = value {
            has_metric = true;
            validate_finite_range(label, value, 0.0, maximum)?;
        }
    }
    has_metric |= metrics.vram_used_mib.is_some() || metrics.concurrency.is_some();
    if metrics
        .concurrency
        .is_some_and(|value| value == 0 || value > 100_000)
    {
        return Err(SessionError::Invalid("concurrency is invalid".to_string()));
    }
    if metrics
        .vram_used_mib
        .is_some_and(|value| value > 128 * 1024 * 1024)
    {
        return Err(SessionError::Invalid(
            "vram_used_mib is invalid".to_string(),
        ));
    }
    if !has_metric {
        return Err(SessionError::Invalid(
            "benchmark result must contain at least one metric".to_string(),
        ));
    }
    Ok(())
}

fn validate_finite_range(
    label: &str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), SessionError> {
    if value.is_finite() && value >= minimum && value <= maximum {
        Ok(())
    } else {
        Err(SessionError::Invalid(format!(
            "{label} must be finite and between {minimum} and {maximum}"
        )))
    }
}

fn validate_digest(label: &str, value: &str) -> Result<(), SessionError> {
    if is_bounded_ascii(value, 256) && value.contains("sha256:") && !contains_secret_text(value) {
        Ok(())
    } else {
        Err(SessionError::Invalid(format!("{label} is invalid")))
    }
}

fn validate_status(value: &str) -> Result<(), SessionError> {
    if matches!(value, "active" | "deprecated" | "disabled") {
        Ok(())
    } else {
        Err(SessionError::Invalid(
            "benchmark profile status must be active, deprecated, or disabled".to_string(),
        ))
    }
}

fn validate_id(label: &str, value: &str, maximum_len: usize) -> Result<(), SessionError> {
    if is_bounded_ascii(value, maximum_len) {
        Ok(())
    } else {
        Err(SessionError::Invalid(format!("{label} is invalid")))
    }
}

fn is_bounded_ascii(value: &str, maximum_len: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum_len
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '_' | '-' | '.' | ':' | '/' | '@')
        })
}

fn parse_rfc3339(label: &str, value: &str) -> Result<DateTime<chrono::FixedOffset>, SessionError> {
    DateTime::parse_from_rfc3339(value)
        .map_err(|error| SessionError::Invalid(format!("{label} must be RFC3339: {error}")))
}

fn validate_json_object(value: &serde_json::Value, label: &str) -> Result<(), SessionError> {
    let normalized = normalized_json_object(value, label)?;
    let encoded = serde_json::to_string(&normalized)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    if encoded.len() > 32 * 1024 || contains_secret_field(&normalized) {
        return Err(SessionError::Invalid(format!(
            "{label} must be small and redacted"
        )));
    }
    Ok(())
}

fn normalized_json_object(
    value: &serde_json::Value,
    label: &str,
) -> Result<serde_json::Value, SessionError> {
    if value.is_null() {
        return Ok(serde_json::json!({}));
    }
    if !matches!(value, serde_json::Value::Object(_)) {
        return Err(SessionError::Invalid(format!(
            "{label} must be a JSON object"
        )));
    }
    Ok(value.clone())
}

fn profile_from_row(row: Row) -> Result<BenchmarkProfileRecord, SessionError> {
    let parameters_json: String = row.get("parameters_json");
    let thresholds_json: String = row.get("thresholds_json");
    Ok(BenchmarkProfileRecord {
        profile_id: row.get("profile_id"),
        profile_version: row.get("profile_version"),
        schema_version: row.get("schema_version"),
        workload_type: row.get("workload_type"),
        display_name: row.get("display_name"),
        description: row.get("description"),
        image_digest: row.get("image_digest"),
        model_hash: row.get("model_hash"),
        artifact_hash: row.get("artifact_hash"),
        required_backend: row.get("required_backend"),
        min_vram_gb: row.get("min_vram_gb"),
        parameters: serde_json::from_str(&parameters_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        warmup_seconds: row.get::<_, i32>("warmup_seconds").max(0) as u32,
        duration_seconds: row.get::<_, i32>("duration_seconds").max(0) as u32,
        sample_count: row.get::<_, i32>("sample_count").max(0) as u32,
        thresholds: serde_json::from_str(&thresholds_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn result_from_row(row: Row) -> Result<BenchmarkResultRecord, SessionError> {
    let parameters_json: String = row.get("parameters_json");
    let metrics_json: String = row.get("metrics_json");
    let verification_json: String = row.get("verification_json");
    let warnings_json: String = row.get("warnings_json");
    Ok(BenchmarkResultRecord {
        result_id: row.get("result_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        run_id: row.get("run_id"),
        profile_id: row.get("profile_id"),
        profile_version: row.get("profile_version"),
        schema_version: row.get("schema_version"),
        workload_type: row.get("workload_type"),
        backend: row.get("backend"),
        hardware_fingerprint: row.get("hardware_fingerprint"),
        gpu_uuid: row.get("gpu_uuid"),
        image_digest: row.get("image_digest"),
        model_hash: row.get("model_hash"),
        artifact_hash: row.get("artifact_hash"),
        parameters: serde_json::from_str(&parameters_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        warmup_seconds: row.get::<_, i32>("warmup_seconds").max(0) as u32,
        duration_seconds: row.get::<_, i32>("duration_seconds").max(0) as u32,
        sample_count: row.get::<_, i32>("sample_count").max(0) as u32,
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        server_received_at: row.get("server_received_at"),
        driver_version: row.get("driver_version"),
        cuda_driver_version: row.get("cuda_driver_version"),
        cuda_runtime_version: row.get("cuda_runtime_version"),
        metrics: serde_json::from_str(&metrics_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        telemetry_window_hash: row.get("telemetry_window_hash"),
        result_hash: row.get("result_hash"),
        public_key_id: row.get("public_key_id"),
        status: row.get("status"),
        verification: serde_json::from_str(&verification_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        warnings: serde_json::from_str(&warnings_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
    })
}

fn profile_select_columns() -> &'static str {
    "SELECT profile_id, profile_version, schema_version, workload_type, display_name, description, image_digest, model_hash, artifact_hash, required_backend, min_vram_gb, parameters_json, warmup_seconds, duration_seconds, sample_count, thresholds_json, status, created_at, updated_at FROM benchmark_profiles"
}

fn result_select_columns() -> &'static str {
    "SELECT result_id, provider_id, device_id, session_id, run_id, profile_id, profile_version, schema_version, workload_type, backend, hardware_fingerprint, gpu_uuid, image_digest, model_hash, artifact_hash, parameters_json, warmup_seconds, duration_seconds, sample_count, started_at, completed_at, server_received_at, driver_version, cuda_driver_version, cuda_runtime_version, metrics_json, telemetry_window_hash, result_hash, public_key_id, status, verification_json, warnings_json FROM benchmark_results"
}

fn contains_secret_field(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .any(|(key, value)| contains_secret_text(key) || contains_secret_field(value)),
        serde_json::Value::Array(items) => items.iter().any(contains_secret_field),
        serde_json::Value::String(value) => contains_secret_text(value),
        _ => false,
    }
}

fn contains_secret_text(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let exact_or_suffix_token = lower == "token"
        || lower.ends_with("_token")
        || lower.ends_with("-token")
        || lower.ends_with(".token");
    exact_or_suffix_token
        || [
            "password",
            "secret",
            "private_key",
            "api_key",
            "authorization",
            "credential",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{
        BenchmarkResultPayload, benchmark_result_hash, benchmark_result_signature_message,
        generate_keypair, sign_message,
    };

    fn profile_request() -> UpsertBenchmarkProfileRequest {
        UpsertBenchmarkProfileRequest {
            profile_id: "llm_realtime_api_small".to_string(),
            profile_version: "2026.07.0".to_string(),
            workload_type: "llm_realtime_api".to_string(),
            display_name: "LLM realtime small".to_string(),
            description: Some("Short prompt inference profile".to_string()),
            image_digest: "ghcr.io/burd/bench-llm@sha256:abc".to_string(),
            model_hash: Some("sha256:model".to_string()),
            artifact_hash: Some("sha256:artifact".to_string()),
            required_backend: "cuda".to_string(),
            min_vram_gb: 8.0,
            parameters: serde_json::json!({"prompt_tokens": 128, "output_tokens": 64}),
            warmup_seconds: 5,
            duration_seconds: 60,
            sample_count: 20,
            thresholds: BenchmarkProfileThresholds {
                min_tokens_per_second: Some(20.0),
                min_sustained_tokens_per_second: Some(18.0),
                max_ttft_ms: Some(500.0),
                ..Default::default()
            },
            status: Some("active".to_string()),
        }
    }

    fn result_payload() -> BenchmarkResultPayload {
        BenchmarkResultPayload {
            schema_version: BENCHMARK_RESULT_SCHEMA_VERSION.to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            run_id: "run_1".to_string(),
            profile_id: "llm_realtime_api_small".to_string(),
            profile_version: "2026.07.0".to_string(),
            workload_type: "llm_realtime_api".to_string(),
            backend: "cuda".to_string(),
            hardware_fingerprint: "sha256:fingerprint".to_string(),
            gpu_uuid: "GPU-test".to_string(),
            image_digest: "ghcr.io/burd/bench-llm@sha256:abc".to_string(),
            model_hash: Some("sha256:model".to_string()),
            artifact_hash: Some("sha256:artifact".to_string()),
            parameters: serde_json::json!({"prompt_tokens": 128, "output_tokens": 64}),
            warmup_seconds: 5,
            duration_seconds: 60,
            sample_count: 20,
            started_at: "2026-07-11T00:00:00Z".to_string(),
            completed_at: "2026-07-11T00:01:00Z".to_string(),
            driver_version: "576.80".to_string(),
            cuda_driver_version: Some("12.9".to_string()),
            cuda_runtime_version: Some("12.8".to_string()),
            metrics: BenchmarkResultMetrics {
                tokens_per_second: Some(42.0),
                sustained_tokens_per_second: Some(38.0),
                ttft_ms: Some(180.0),
                latency_p50_ms: Some(600.0),
                latency_p95_ms: Some(900.0),
                ..Default::default()
            },
            telemetry_window_hash: Some("sha256:telemetry".to_string()),
            warnings: Vec::new(),
        }
    }

    fn signed_result(public_key_id: &str, secret_key: &str) -> SignedBenchmarkResult {
        let payload = result_payload();
        let hash = benchmark_result_hash(&payload).unwrap();
        let message = benchmark_result_signature_message(&payload, &hash, public_key_id).unwrap();
        SignedBenchmarkResult {
            payload,
            result_hash: hash,
            public_key_id: public_key_id.to_string(),
            signature: sign_message(secret_key, message.as_bytes()).unwrap(),
            canonicalization_version: BENCHMARK_RESULT_CANONICALIZATION_VERSION.to_string(),
        }
    }

    #[test]
    fn validation_rejects_secret_parameters_without_rejecting_token_counts() {
        let request = profile_request();
        assert!(validate_profile_request(&request).is_ok());

        let mut secret_request = profile_request();
        secret_request.parameters = serde_json::json!({"api_token": "leak"});
        assert!(validate_profile_request(&secret_request).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn persists_profile_and_signed_result() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let keys = generate_keypair().unwrap();
        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        let expires_at = (Utc::now() + Duration::minutes(30)).to_rfc3339();
        client
            .execute(
                "INSERT INTO providers (provider_id, user_id, display_name, status, created_at, updated_at) VALUES ($1, NULL, $2, 'available', $3, $3)",
                &[&"provider_1", &"Benchmark Provider", &now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ($1, $2, $3, 'active', $4, $4)",
                &[&"device_1", &"provider_1", &"machine_1", &now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at) VALUES ($1, $2, $3, $4, 'ed25519', 'active', $5)",
                &[&"key_1", &"provider_1", &"device_1", &keys.public_key_base64, &now],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint) VALUES ($1, $2, $3, 'online', 0, $4, $5, $6)",
                &[&"session_1", &"provider_1", &"device_1", &now, &expires_at, &"sha256:fingerprint"],
            )
            .await
            .unwrap();

        let profile = db
            .upsert_benchmark_profile("req_profile", &profile_request())
            .await
            .unwrap()
            .profile;
        assert_eq!(profile.profile_version, "2026.07.0");

        let authorized = AuthorizedSession {
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            sequence_last: 0,
            heartbeat_interval_seconds: 30,
            missed_heartbeat_limit: 3,
        };
        let signed = signed_result("key_1", &keys.secret_key_base64);
        let response = db
            .submit_benchmark_result("req_result", &authorized, &signed)
            .await
            .unwrap();
        assert!(!response.duplicate);
        assert_eq!(response.result.status, "succeeded");
        assert!(response.result.verification.metrics_satisfied);

        let replay = db
            .submit_benchmark_result("req_result_replay", &authorized, &signed)
            .await
            .unwrap();
        assert!(replay.duplicate);

        let results = db
            .list_provider_benchmark_results("req_list", "provider_1", 10)
            .await
            .unwrap();
        assert_eq!(results.results.len(), 1);
        db.drop_schema_for_test().await.unwrap();
    }
}
