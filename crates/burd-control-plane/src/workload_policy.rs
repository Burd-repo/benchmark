use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::SessionError;
use burd_protocol::{
    BenchmarkResultMetrics, ListProviderWorkloadEligibilityResponse, ListWorkloadPoliciesResponse,
    RegionalReachability, RunWorkloadEligibilityRequest, RunWorkloadEligibilityResponse,
    UpsertWorkloadPolicyRequest, UpsertWorkloadPolicyResponse, WORKLOAD_ELIGIBILITY_SCHEMA_VERSION,
    WORKLOAD_POLICY_ENGINE_VERSION, WORKLOAD_POLICY_SCHEMA_VERSION, WorkloadEligibilityRecord,
    WorkloadPolicyRecord, WorkloadPolicyRequirements,
};
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::Row;

const WORKLOAD_ELIGIBILITY_SWEEP_LIMIT: u32 = 500;
const MAX_POLICY_DESCRIPTION_LEN: usize = 512;

#[derive(Debug, Clone)]
struct WorkloadCandidate {
    provider_id: String,
    provider_status: String,
    device_id: String,
    device_status: String,
    policy_id: String,
    policy_version: String,
    workload_type: String,
    policy_status: String,
    requirements: WorkloadPolicyRequirements,
    trust_status: Option<String>,
    trust_score: Option<f64>,
    risk_score: Option<f64>,
    reliability_score: Option<f64>,
    verification_status: Option<String>,
    last_verified_at: Option<String>,
    remote_network_score: Option<f64>,
    regional_reachability: Vec<RegionalReachability>,
    session_status: Option<String>,
    session_hardware_fingerprint: Option<String>,
    latest_gpu_uuid: Option<String>,
    vram_total_mib: Option<i64>,
    benchmark_result_id: Option<String>,
    benchmark_profile_id: Option<String>,
    benchmark_profile_version: Option<String>,
    benchmark_backend: Option<String>,
    benchmark_hardware_fingerprint: Option<String>,
    benchmark_gpu_uuid: Option<String>,
    benchmark_completed_at: Option<String>,
    benchmark_status: Option<String>,
    benchmark_metrics: Option<BenchmarkResultMetrics>,
}

#[derive(Debug, Clone)]
struct EligibilityEvaluation {
    status: String,
    reason_codes: Vec<String>,
}

impl Database {
    pub async fn upsert_workload_policy(
        &self,
        request_id: &str,
        request: &UpsertWorkloadPolicyRequest,
    ) -> Result<UpsertWorkloadPolicyResponse, SessionError> {
        validate_workload_policy_request(request)?;
        let requirements_json = serde_json::to_string(&request.requirements)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let status = request.status.as_deref().unwrap_or("active").to_string();
        let now = Utc::now().to_rfc3339();

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        transaction
            .execute(
                "INSERT INTO workload_policies (policy_id, policy_version, schema_version, workload_type, display_name, description, requirements_json, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) ON CONFLICT (policy_id, policy_version) DO UPDATE SET workload_type = EXCLUDED.workload_type, display_name = EXCLUDED.display_name, description = EXCLUDED.description, requirements_json = EXCLUDED.requirements_json, status = EXCLUDED.status, updated_at = EXCLUDED.updated_at",
                &[
                    &request.policy_id,
                    &request.policy_version,
                    &WORKLOAD_POLICY_SCHEMA_VERSION,
                    &request.workload_type,
                    &request.display_name,
                    &request.description,
                    &requirements_json,
                    &status,
                    &now,
                ],
            )
            .await?;
        let row = transaction
            .query_one(
                &format!(
                    "{} WHERE policy_id = $1 AND policy_version = $2",
                    policy_select_columns()
                ),
                &[&request.policy_id, &request.policy_version],
            )
            .await?;
        let policy = policy_from_row(row)?;
        let audit_metadata = serde_json::json!({
            "policy_id": policy.policy_id,
            "policy_version": policy.policy_version,
            "workload_type": policy.workload_type,
            "status": policy.status,
        })
        .to_string();
        insert_audit_event(
            &transaction,
            NewAuditEvent {
                request_id,
                actor_type: "admin",
                actor_id: None,
                entity_type: "workload_policy",
                entity_id: &policy.policy_id,
                event_type: "workload_policy.upserted",
                idempotency_key: None,
                summary: "workload eligibility policy v2 upserted",
                metadata_json: &audit_metadata,
            },
        )
        .await?;
        transaction.commit().await?;

        Ok(UpsertWorkloadPolicyResponse {
            request_id: request_id.to_string(),
            policy,
        })
    }

    pub async fn list_workload_policies(
        &self,
        request_id: &str,
    ) -> Result<ListWorkloadPoliciesResponse, SessionError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} ORDER BY workload_type, policy_id, policy_version DESC",
                    policy_select_columns()
                ),
                &[],
            )
            .await?;
        let policies = rows
            .into_iter()
            .map(policy_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListWorkloadPoliciesResponse {
            request_id: request_id.to_string(),
            policies,
        })
    }

    pub async fn run_workload_eligibility_sweep(
        &self,
        request_id: &str,
        request: &RunWorkloadEligibilityRequest,
    ) -> Result<RunWorkloadEligibilityResponse, SessionError> {
        validate_eligibility_sweep_request(request)?;
        let limit = request
            .limit
            .unwrap_or(WORKLOAD_ELIGIBILITY_SWEEP_LIMIT)
            .min(WORKLOAD_ELIGIBILITY_SWEEP_LIMIT);
        let candidates = self.workload_candidates(limit).await?;
        let evaluated = candidates.len() as u32;
        let now = Utc::now();
        let mut updated = Vec::with_capacity(candidates.len());

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        for candidate in candidates {
            let evaluation = evaluate_workload_candidate(&candidate, now);
            let timestamp = now.to_rfc3339();
            let reason_codes_json = serde_json::to_string(&evaluation.reason_codes)
                .map_err(|error| SessionError::Invalid(error.to_string()))?;
            let hardware_fingerprint = candidate
                .session_hardware_fingerprint
                .clone()
                .or(candidate.benchmark_hardware_fingerprint.clone());
            let regional_reachability_json =
                serde_json::to_string(&candidate.regional_reachability)
                    .map_err(|error| SessionError::Invalid(error.to_string()))?;
            transaction
                .execute(
                    "INSERT INTO provider_workload_eligibility (provider_id, device_id, workload_type, policy_id, policy_version, schema_version, engine_version, status, reason_codes_json, trust_score, risk_score, reliability_score, verification_status, remote_network_score, benchmark_result_id, benchmark_profile_id, benchmark_profile_version, benchmark_backend, benchmark_completed_at, benchmark_status, session_status, latest_gpu_uuid, vram_total_mib, hardware_fingerprint, regional_reachability_json, evaluated_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $26) ON CONFLICT (provider_id, device_id, policy_id, policy_version) DO UPDATE SET workload_type = EXCLUDED.workload_type, schema_version = EXCLUDED.schema_version, engine_version = EXCLUDED.engine_version, status = EXCLUDED.status, reason_codes_json = EXCLUDED.reason_codes_json, trust_score = EXCLUDED.trust_score, risk_score = EXCLUDED.risk_score, reliability_score = EXCLUDED.reliability_score, verification_status = EXCLUDED.verification_status, remote_network_score = EXCLUDED.remote_network_score, benchmark_result_id = EXCLUDED.benchmark_result_id, benchmark_profile_id = EXCLUDED.benchmark_profile_id, benchmark_profile_version = EXCLUDED.benchmark_profile_version, benchmark_backend = EXCLUDED.benchmark_backend, benchmark_completed_at = EXCLUDED.benchmark_completed_at, benchmark_status = EXCLUDED.benchmark_status, session_status = EXCLUDED.session_status, latest_gpu_uuid = EXCLUDED.latest_gpu_uuid, vram_total_mib = EXCLUDED.vram_total_mib, hardware_fingerprint = EXCLUDED.hardware_fingerprint, regional_reachability_json = EXCLUDED.regional_reachability_json, evaluated_at = EXCLUDED.evaluated_at, updated_at = EXCLUDED.updated_at",
                    &[
                        &candidate.provider_id,
                        &candidate.device_id,
                        &candidate.workload_type,
                        &candidate.policy_id,
                        &candidate.policy_version,
                        &WORKLOAD_ELIGIBILITY_SCHEMA_VERSION,
                        &WORKLOAD_POLICY_ENGINE_VERSION,
                        &evaluation.status,
                        &reason_codes_json,
                        &candidate.trust_score,
                        &candidate.risk_score,
                        &candidate.reliability_score,
                        &candidate.verification_status,
                        &candidate.remote_network_score,
                        &candidate.benchmark_result_id,
                        &candidate.benchmark_profile_id,
                        &candidate.benchmark_profile_version,
                        &candidate.benchmark_backend,
                        &candidate.benchmark_completed_at,
                        &candidate.benchmark_status,
                        &candidate.session_status,
                        &candidate.latest_gpu_uuid,
                        &candidate.vram_total_mib,
                        &hardware_fingerprint,
                        &regional_reachability_json,
                        &timestamp,
                    ],
                )
                .await?;
            let row = transaction
                .query_one(
                    &format!(
                        "{} WHERE provider_id = $1 AND device_id = $2 AND policy_id = $3 AND policy_version = $4",
                        eligibility_select_columns()
                    ),
                    &[
                        &candidate.provider_id,
                        &candidate.device_id,
                        &candidate.policy_id,
                        &candidate.policy_version,
                    ],
                )
                .await?;
            let state = eligibility_from_row(row)?;
            let audit_metadata = serde_json::json!({
                "device_id": state.device_id,
                "workload_type": state.workload_type,
                "policy_id": state.policy_id,
                "policy_version": state.policy_version,
                "status": state.status,
                "reason_codes": state.reason_codes,
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "system",
                    actor_id: None,
                    entity_type: "provider_workload_eligibility",
                    entity_id: &state.provider_id,
                    event_type: "workload_eligibility.recalculated",
                    idempotency_key: None,
                    summary: "backend workload eligibility recalculated",
                    metadata_json: &audit_metadata,
                },
            )
            .await?;
            updated.push(state);
        }
        transaction.commit().await?;

        Ok(RunWorkloadEligibilityResponse {
            request_id: request_id.to_string(),
            evaluated,
            updated,
        })
    }

    pub async fn list_provider_workload_eligibility(
        &self,
        request_id: &str,
        provider_id: &str,
    ) -> Result<ListProviderWorkloadEligibilityResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE provider_id = $1 ORDER BY workload_type, status, updated_at DESC",
                    eligibility_select_columns()
                ),
                &[&provider_id],
            )
            .await?;
        let states = rows
            .into_iter()
            .map(eligibility_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListProviderWorkloadEligibilityResponse {
            request_id: request_id.to_string(),
            states,
        })
    }

    async fn workload_candidates(
        &self,
        limit: u32,
    ) -> Result<Vec<WorkloadCandidate>, SessionError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT p.provider_id, p.status AS provider_status, d.device_id, d.status AS device_status, wp.policy_id, wp.policy_version, wp.workload_type, wp.status AS policy_status, wp.requirements_json, ts.status AS trust_status, ts.trust_score, ts.risk_score, ts.reliability_score, COALESCE(ts.verification_status, vs.status) AS verification_status, vs.last_verified_at, COALESCE(ns.effective_network_score, ns.remote_network_score, ts.remote_network_score) AS remote_network_score, ns.regional_reachability_json, s.status AS session_status, s.hardware_fingerprint AS session_hardware_fingerprint, COALESCE(ts.latest_gpu_uuid, latest_gpu.gpu_uuid, br.gpu_uuid) AS latest_gpu_uuid, latest_gpu.vram_total_mib, br.result_id AS benchmark_result_id, br.profile_id AS benchmark_profile_id, br.profile_version AS benchmark_profile_version, br.backend AS benchmark_backend, br.hardware_fingerprint AS benchmark_hardware_fingerprint, br.gpu_uuid AS benchmark_gpu_uuid, br.completed_at AS benchmark_completed_at, br.status AS benchmark_status, br.metrics_json AS benchmark_metrics_json FROM workload_policies wp JOIN devices d ON d.status IN ('active', 'revoked') JOIN providers p ON p.provider_id = d.provider_id LEFT JOIN provider_trust_states ts ON ts.provider_id = d.provider_id AND ts.device_id = d.device_id LEFT JOIN provider_verification_states vs ON vs.provider_id = d.provider_id AND vs.device_id = d.device_id LEFT JOIN provider_network_states ns ON ns.provider_id = d.provider_id AND ns.device_id = d.device_id LEFT JOIN LATERAL (SELECT session_id, status, hardware_fingerprint, started_at FROM provider_sessions ps WHERE ps.provider_id = d.provider_id AND ps.device_id = d.device_id ORDER BY started_at DESC LIMIT 1) s ON TRUE LEFT JOIN LATERAL (SELECT gpu_uuid, vram_total_mib FROM gpu_telemetry_samples gs WHERE gs.provider_id = d.provider_id AND gs.device_id = d.device_id ORDER BY server_received_at DESC LIMIT 1) latest_gpu ON TRUE LEFT JOIN LATERAL (SELECT result_id, profile_id, profile_version, backend, hardware_fingerprint, gpu_uuid, completed_at, status, metrics_json, server_received_at FROM benchmark_results br WHERE br.provider_id = d.provider_id AND br.device_id = d.device_id AND br.workload_type = wp.workload_type AND (COALESCE(wp.requirements_json::jsonb ->> 'benchmark_profile_id', '') = '' OR br.profile_id = wp.requirements_json::jsonb ->> 'benchmark_profile_id') AND (COALESCE(wp.requirements_json::jsonb ->> 'benchmark_profile_version', '') = '' OR br.profile_version = wp.requirements_json::jsonb ->> 'benchmark_profile_version') ORDER BY br.completed_at DESC, br.server_received_at DESC LIMIT 1) br ON TRUE WHERE wp.status = 'active' ORDER BY p.updated_at DESC, d.updated_at DESC, wp.workload_type, wp.policy_id LIMIT $1",
                &[&(limit as i64)],
            )
            .await?;
        rows.into_iter().map(candidate_from_row).collect()
    }
}

fn evaluate_workload_candidate(
    candidate: &WorkloadCandidate,
    now: DateTime<Utc>,
) -> EligibilityEvaluation {
    let mut reason_codes = Vec::new();
    let mut rank = 0_u8;
    if candidate.policy_status != "active" {
        reason_codes.push("policy_inactive".to_string());
        raise_rank(&mut rank, 5);
    }
    if matches!(
        candidate.provider_status.as_str(),
        "blocked" | "quarantined"
    ) || candidate.device_status != "active"
    {
        reason_codes.push("provider_or_device_blocked".to_string());
        raise_rank(&mut rank, 5);
    }

    match candidate.trust_status.as_deref() {
        Some("blocked" | "quarantined") => {
            reason_codes.push("trust_state_blocked".to_string());
            raise_rank(&mut rank, 5);
        }
        Some("suspect") => {
            reason_codes.push("trust_state_suspect".to_string());
            raise_rank(&mut rank, 3);
        }
        Some("degraded") => {
            reason_codes.push("trust_state_degraded".to_string());
            raise_rank(&mut rank, 2);
        }
        Some("new_provider" | "insufficient_history") => {
            reason_codes.push("trust_history_insufficient".to_string());
            raise_rank(&mut rank, 4);
        }
        Some("trusted" | "highly_trusted") => reason_codes.push("trust_state_accepted".to_string()),
        Some(other) => {
            reason_codes.push(format!("trust_state_{other}"));
            raise_rank(&mut rank, 4);
        }
        None => {
            reason_codes.push("trust_state_missing".to_string());
            raise_rank(&mut rank, 4);
        }
    }

    if let Some(minimum) = candidate.requirements.min_trust_score {
        if let Some(score) = candidate.trust_score {
            if score < minimum {
                reason_codes.push("trust_score_below_policy".to_string());
                raise_rank(&mut rank, 3);
            }
        } else {
            reason_codes.push("trust_score_missing".to_string());
            raise_rank(&mut rank, 4);
        }
    }
    if let Some(maximum) = candidate.requirements.max_risk_score {
        if let Some(score) = candidate.risk_score {
            if score > maximum {
                reason_codes.push("risk_score_above_policy".to_string());
                raise_rank(&mut rank, 3);
            }
        } else {
            reason_codes.push("risk_score_missing".to_string());
            raise_rank(&mut rank, 4);
        }
    }
    if let Some(minimum) = candidate.requirements.min_reliability_score {
        if let Some(score) = candidate.reliability_score {
            if score < minimum {
                reason_codes.push("reliability_below_policy".to_string());
                raise_rank(&mut rank, 2);
            }
        } else {
            reason_codes.push("reliability_missing".to_string());
            raise_rank(&mut rank, 2);
        }
    }

    match candidate.session_status.as_deref() {
        Some("online") => reason_codes.push("session_online".to_string()),
        Some("degraded") => {
            reason_codes.push("session_degraded".to_string());
            raise_rank(&mut rank, 2);
        }
        Some("offline" | "expired" | "revoked" | "pending_connection") => {
            reason_codes.push("session_not_available".to_string());
            raise_rank(&mut rank, 1);
        }
        _ => {
            reason_codes.push("session_missing".to_string());
            raise_rank(&mut rank, 1);
        }
    }

    if let Some(required) = candidate
        .requirements
        .required_verification_status
        .as_deref()
    {
        if candidate.verification_status.as_deref() == Some(required) {
            reason_codes.push("verification_status_satisfied".to_string());
        } else {
            reason_codes.push("verification_status_not_satisfied".to_string());
            raise_rank(&mut rank, 4);
        }
    } else if candidate.verification_status.is_none() {
        reason_codes.push("verification_state_missing".to_string());
        raise_rank(&mut rank, 4);
    }
    if let Some(max_age) = candidate.requirements.recent_proof_max_age_seconds {
        match candidate.last_verified_at.as_deref() {
            Some(last_verified_at)
                if !timestamp_older_than(last_verified_at, max_age, now).unwrap_or(true) =>
            {
                reason_codes.push("recent_proof_fresh".to_string());
            }
            Some(_) => {
                reason_codes.push("recent_proof_stale".to_string());
                raise_rank(&mut rank, 4);
            }
            None => {
                reason_codes.push("recent_proof_missing".to_string());
                raise_rank(&mut rank, 4);
            }
        }
    }

    if let Some(minimum) = candidate.requirements.min_remote_network_score {
        if let Some(score) = candidate.remote_network_score {
            if score < minimum {
                reason_codes.push("remote_network_below_policy".to_string());
                raise_rank(&mut rank, 2);
            }
        } else {
            reason_codes.push("remote_network_missing".to_string());
            raise_rank(&mut rank, 2);
        }
    }

    if let Some(min_vram_gb) = candidate.requirements.min_vram_gb {
        let required_mib = min_vram_gb * 1024.0;
        match candidate.vram_total_mib {
            Some(vram_total_mib) if (vram_total_mib as f64) >= required_mib => {
                reason_codes.push("vram_requirement_satisfied".to_string());
            }
            Some(_) => {
                reason_codes.push("vram_below_policy".to_string());
                raise_rank(&mut rank, 3);
            }
            None => {
                reason_codes.push("vram_unobserved".to_string());
                raise_rank(&mut rank, 4);
            }
        }
    }
    if candidate.requirements.gpu_family.is_some() {
        reason_codes.push("gpu_family_attestation_unavailable".to_string());
        raise_rank(&mut rank, 4);
    }
    evaluate_region_requirement(candidate, &mut reason_codes, &mut rank);
    evaluate_benchmark_requirements(candidate, now, &mut reason_codes, &mut rank);

    if rank == 0 {
        reason_codes.push("policy_requirements_satisfied".to_string());
    }
    reason_codes.sort();
    reason_codes.dedup();

    let status = match rank {
        5 => "blocked",
        4 => "verification_required",
        3 => "ineligible",
        2 => "limited",
        1 => "temporarily_unavailable",
        _ => "eligible",
    }
    .to_string();

    EligibilityEvaluation {
        status,
        reason_codes,
    }
}

fn raise_rank(rank: &mut u8, new_rank: u8) {
    if new_rank > *rank {
        *rank = new_rank;
    }
}

fn evaluate_region_requirement(
    candidate: &WorkloadCandidate,
    reason_codes: &mut Vec<String>,
    rank: &mut u8,
) {
    if candidate.requirements.allowed_regions.is_empty() {
        return;
    }
    let allowed = candidate
        .requirements
        .allowed_regions
        .iter()
        .map(|region| region.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let observed = candidate
        .regional_reachability
        .iter()
        .filter(|reachability| reachability.status != "unreachable")
        .flat_map(|reachability| {
            [
                Some(reachability.probe_region.as_str()),
                reachability.approximate_region.as_deref(),
            ]
        })
        .flatten()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if observed.is_empty() {
        reason_codes.push("region_unobserved".to_string());
        raise_rank(rank, 2);
    } else if observed.iter().any(|region| allowed.contains(region)) {
        reason_codes.push("region_requirement_satisfied".to_string());
    } else {
        reason_codes.push("region_not_allowed".to_string());
        raise_rank(rank, 3);
    }
}

fn evaluate_benchmark_requirements(
    candidate: &WorkloadCandidate,
    now: DateTime<Utc>,
    reason_codes: &mut Vec<String>,
    rank: &mut u8,
) {
    let requirements = &candidate.requirements;
    let benchmark_required = requirements.benchmark_profile_id.is_some()
        || requirements.benchmark_profile_version.is_some()
        || requirements.benchmark_max_age_seconds.is_some()
        || requirements.required_backend.is_some()
        || requirements.min_tokens_per_second.is_some()
        || requirements.min_sustained_tokens_per_second.is_some()
        || requirements.min_requests_per_second.is_some()
        || requirements.max_ttft_ms.is_some()
        || requirements.max_latency_p95_ms.is_some();
    if !benchmark_required {
        return;
    }
    if candidate.benchmark_result_id.is_none() {
        reason_codes.push("benchmark_result_missing".to_string());
        raise_rank(rank, 4);
        return;
    }
    if candidate.benchmark_status.as_deref() != Some("succeeded") {
        reason_codes.push("benchmark_not_succeeded".to_string());
        raise_rank(rank, 3);
    }
    if let Some(required_backend) = requirements.required_backend.as_deref() {
        if candidate.benchmark_backend.as_deref() == Some(required_backend) {
            reason_codes.push("benchmark_backend_satisfied".to_string());
        } else {
            reason_codes.push("benchmark_backend_mismatch".to_string());
            raise_rank(rank, 3);
        }
    }
    if let Some(profile_id) = requirements.benchmark_profile_id.as_deref()
        && candidate.benchmark_profile_id.as_deref() != Some(profile_id)
    {
        reason_codes.push("benchmark_profile_mismatch".to_string());
        raise_rank(rank, 4);
    }
    if let Some(profile_version) = requirements.benchmark_profile_version.as_deref()
        && candidate.benchmark_profile_version.as_deref() != Some(profile_version)
    {
        reason_codes.push("benchmark_profile_version_mismatch".to_string());
        raise_rank(rank, 4);
    }
    if let Some(max_age) = requirements.benchmark_max_age_seconds {
        match candidate.benchmark_completed_at.as_deref() {
            Some(completed_at)
                if !timestamp_older_than(completed_at, max_age, now).unwrap_or(true) =>
            {
                reason_codes.push("benchmark_fresh".to_string());
            }
            Some(_) => {
                reason_codes.push("benchmark_stale".to_string());
                raise_rank(rank, 4);
            }
            None => {
                reason_codes.push("benchmark_completed_at_missing".to_string());
                raise_rank(rank, 4);
            }
        }
    }
    if let (Some(session_fingerprint), Some(benchmark_fingerprint)) = (
        candidate.session_hardware_fingerprint.as_deref(),
        candidate.benchmark_hardware_fingerprint.as_deref(),
    ) && session_fingerprint != benchmark_fingerprint
    {
        reason_codes.push("benchmark_fingerprint_mismatch".to_string());
        raise_rank(rank, 4);
    }
    if let (Some(latest_gpu_uuid), Some(benchmark_gpu_uuid)) = (
        candidate.latest_gpu_uuid.as_deref(),
        candidate.benchmark_gpu_uuid.as_deref(),
    ) && latest_gpu_uuid != benchmark_gpu_uuid
    {
        reason_codes.push("benchmark_gpu_mismatch".to_string());
        raise_rank(rank, 4);
    }
    let Some(metrics) = candidate.benchmark_metrics.as_ref() else {
        reason_codes.push("benchmark_metrics_missing".to_string());
        raise_rank(rank, 4);
        return;
    };
    enforce_minimum_metric(
        metrics.tokens_per_second,
        requirements.min_tokens_per_second,
        "tokens_per_second",
        reason_codes,
        rank,
    );
    enforce_minimum_metric(
        metrics.sustained_tokens_per_second,
        requirements.min_sustained_tokens_per_second,
        "sustained_tokens_per_second",
        reason_codes,
        rank,
    );
    enforce_minimum_metric(
        metrics.requests_per_second,
        requirements.min_requests_per_second,
        "requests_per_second",
        reason_codes,
        rank,
    );
    enforce_maximum_metric(
        metrics.ttft_ms,
        requirements.max_ttft_ms,
        "ttft_ms",
        reason_codes,
        rank,
    );
    enforce_maximum_metric(
        metrics.latency_p95_ms,
        requirements.max_latency_p95_ms,
        "latency_p95_ms",
        reason_codes,
        rank,
    );
}

fn enforce_minimum_metric(
    actual: Option<f64>,
    required: Option<f64>,
    metric: &str,
    reason_codes: &mut Vec<String>,
    rank: &mut u8,
) {
    if let Some(required) = required {
        match actual {
            Some(actual) if actual >= required => reason_codes.push(format!("{metric}_satisfied")),
            Some(_) => {
                reason_codes.push(format!("{metric}_below_policy"));
                raise_rank(rank, 3);
            }
            None => {
                reason_codes.push(format!("{metric}_missing"));
                raise_rank(rank, 4);
            }
        }
    }
}

fn enforce_maximum_metric(
    actual: Option<f64>,
    maximum: Option<f64>,
    metric: &str,
    reason_codes: &mut Vec<String>,
    rank: &mut u8,
) {
    if let Some(maximum) = maximum {
        match actual {
            Some(actual) if actual <= maximum => reason_codes.push(format!("{metric}_satisfied")),
            Some(_) => {
                reason_codes.push(format!("{metric}_above_policy"));
                raise_rank(rank, 3);
            }
            None => {
                reason_codes.push(format!("{metric}_missing"));
                raise_rank(rank, 4);
            }
        }
    }
}

fn timestamp_older_than(
    timestamp: &str,
    max_age_seconds: u32,
    now: DateTime<Utc>,
) -> Result<bool, SessionError> {
    let parsed = DateTime::parse_from_rfc3339(timestamp)
        .map_err(|_| SessionError::Invalid("timestamp must be RFC3339".to_string()))?
        .with_timezone(&Utc);
    Ok(now.signed_duration_since(parsed) > Duration::seconds(i64::from(max_age_seconds)))
}

fn policy_from_row(row: Row) -> Result<WorkloadPolicyRecord, SessionError> {
    let requirements_json: String = row.get("requirements_json");
    Ok(WorkloadPolicyRecord {
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
        schema_version: row.get("schema_version"),
        workload_type: row.get("workload_type"),
        display_name: row.get("display_name"),
        description: row.get("description"),
        requirements: serde_json::from_str(&requirements_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn candidate_from_row(row: Row) -> Result<WorkloadCandidate, SessionError> {
    let requirements_json: String = row.get("requirements_json");
    let metrics_json: Option<String> = row.get("benchmark_metrics_json");
    let regional_reachability_json: Option<String> = row.get("regional_reachability_json");
    Ok(WorkloadCandidate {
        provider_id: row.get("provider_id"),
        provider_status: row.get("provider_status"),
        device_id: row.get("device_id"),
        device_status: row.get("device_status"),
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
        workload_type: row.get("workload_type"),
        policy_status: row.get("policy_status"),
        requirements: serde_json::from_str(&requirements_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        trust_status: row.get("trust_status"),
        trust_score: row.get("trust_score"),
        risk_score: row.get("risk_score"),
        reliability_score: row.get("reliability_score"),
        verification_status: row.get("verification_status"),
        last_verified_at: row.get("last_verified_at"),
        remote_network_score: row.get("remote_network_score"),
        regional_reachability: serde_json::from_str(
            regional_reachability_json.as_deref().unwrap_or("[]"),
        )
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        session_status: row.get("session_status"),
        session_hardware_fingerprint: row.get("session_hardware_fingerprint"),
        latest_gpu_uuid: row.get("latest_gpu_uuid"),
        vram_total_mib: row.get("vram_total_mib"),
        benchmark_result_id: row.get("benchmark_result_id"),
        benchmark_profile_id: row.get("benchmark_profile_id"),
        benchmark_profile_version: row.get("benchmark_profile_version"),
        benchmark_backend: row.get("benchmark_backend"),
        benchmark_hardware_fingerprint: row.get("benchmark_hardware_fingerprint"),
        benchmark_gpu_uuid: row.get("benchmark_gpu_uuid"),
        benchmark_completed_at: row.get("benchmark_completed_at"),
        benchmark_status: row.get("benchmark_status"),
        benchmark_metrics: metrics_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
    })
}

fn eligibility_from_row(row: Row) -> Result<WorkloadEligibilityRecord, SessionError> {
    let reason_codes_json: String = row.get("reason_codes_json");
    Ok(WorkloadEligibilityRecord {
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        workload_type: row.get("workload_type"),
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
        schema_version: row.get("schema_version"),
        engine_version: row.get("engine_version"),
        status: row.get("status"),
        reason_codes: serde_json::from_str(&reason_codes_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        trust_score: row.get("trust_score"),
        risk_score: row.get("risk_score"),
        reliability_score: row.get("reliability_score"),
        verification_status: row.get("verification_status"),
        remote_network_score: row.get("remote_network_score"),
        benchmark_result_id: row.get("benchmark_result_id"),
        benchmark_profile_id: row.get("benchmark_profile_id"),
        benchmark_profile_version: row.get("benchmark_profile_version"),
        benchmark_completed_at: row.get("benchmark_completed_at"),
        benchmark_status: row.get("benchmark_status"),
        session_status: row.get("session_status"),
        latest_gpu_uuid: row.get("latest_gpu_uuid"),
        hardware_fingerprint: row.get("hardware_fingerprint"),
        evaluated_at: row.get("evaluated_at"),
        updated_at: row.get("updated_at"),
    })
}

fn policy_select_columns() -> &'static str {
    "SELECT policy_id, policy_version, schema_version, workload_type, display_name, description, requirements_json, status, created_at, updated_at FROM workload_policies"
}

fn eligibility_select_columns() -> &'static str {
    "SELECT provider_id, device_id, workload_type, policy_id, policy_version, schema_version, engine_version, status, reason_codes_json, trust_score, risk_score, reliability_score, verification_status, remote_network_score, benchmark_result_id, benchmark_profile_id, benchmark_profile_version, benchmark_completed_at, benchmark_status, session_status, latest_gpu_uuid, hardware_fingerprint, evaluated_at, updated_at FROM provider_workload_eligibility"
}

fn validate_workload_policy_request(
    request: &UpsertWorkloadPolicyRequest,
) -> Result<(), SessionError> {
    validate_id("policy_id", &request.policy_id, 128)?;
    validate_id("policy_version", &request.policy_version, 64)?;
    validate_id("workload_type", &request.workload_type, 96)?;
    if !is_bounded_ascii(&request.display_name, 120) {
        return Err(SessionError::Invalid(
            "workload policy display_name must be short printable ASCII".to_string(),
        ));
    }
    if let Some(description) = request.description.as_deref()
        && !is_bounded_ascii(description, MAX_POLICY_DESCRIPTION_LEN)
    {
        return Err(SessionError::Invalid(
            "workload policy description must be printable ASCII".to_string(),
        ));
    }
    if let Some(status) = request.status.as_deref() {
        validate_status(status)?;
    }
    validate_requirements(&request.requirements)
}

fn validate_requirements(requirements: &WorkloadPolicyRequirements) -> Result<(), SessionError> {
    if let Some(gpu_family) = requirements.gpu_family.as_deref()
        && !is_bounded_ascii(gpu_family, 96)
    {
        return Err(SessionError::Invalid(
            "gpu_family must be short printable ASCII".to_string(),
        ));
    }
    if let Some(required_backend) = requirements.required_backend.as_deref() {
        validate_id("required_backend", required_backend, 64)?;
    }
    if let Some(profile_id) = requirements.benchmark_profile_id.as_deref() {
        validate_id("benchmark_profile_id", profile_id, 128)?;
    }
    if let Some(profile_version) = requirements.benchmark_profile_version.as_deref() {
        validate_id("benchmark_profile_version", profile_version, 64)?;
    }
    if let Some(status) = requirements.required_verification_status.as_deref() {
        validate_id("required_verification_status", status, 64)?;
    }
    for region in &requirements.allowed_regions {
        validate_id("allowed_region", region, 64)?;
    }
    validate_finite_range("min_vram_gb", requirements.min_vram_gb, 0.0, 1024.0)?;
    validate_finite_range(
        "min_tokens_per_second",
        requirements.min_tokens_per_second,
        0.0,
        1_000_000.0,
    )?;
    validate_finite_range(
        "min_sustained_tokens_per_second",
        requirements.min_sustained_tokens_per_second,
        0.0,
        1_000_000.0,
    )?;
    validate_finite_range(
        "min_requests_per_second",
        requirements.min_requests_per_second,
        0.0,
        1_000_000.0,
    )?;
    validate_finite_range("max_ttft_ms", requirements.max_ttft_ms, 0.0, 600_000.0)?;
    validate_finite_range(
        "max_latency_p95_ms",
        requirements.max_latency_p95_ms,
        0.0,
        600_000.0,
    )?;
    validate_finite_range("min_trust_score", requirements.min_trust_score, 0.0, 100.0)?;
    validate_finite_range("max_risk_score", requirements.max_risk_score, 0.0, 100.0)?;
    validate_finite_range(
        "min_reliability_score",
        requirements.min_reliability_score,
        0.0,
        100.0,
    )?;
    validate_finite_range(
        "min_remote_network_score",
        requirements.min_remote_network_score,
        0.0,
        100.0,
    )?;
    validate_finite_range(
        "max_price_per_hour",
        requirements.max_price_per_hour,
        0.0,
        1_000_000.0,
    )?;
    validate_age(
        "benchmark_max_age_seconds",
        requirements.benchmark_max_age_seconds,
    )?;
    validate_age(
        "recent_proof_max_age_seconds",
        requirements.recent_proof_max_age_seconds,
    )?;
    let requirements_json = serde_json::to_value(requirements)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    if contains_secret_field(&requirements_json) {
        return Err(SessionError::Invalid(
            "workload policy requirements must not contain secrets".to_string(),
        ));
    }
    Ok(())
}

fn validate_eligibility_sweep_request(
    request: &RunWorkloadEligibilityRequest,
) -> Result<(), SessionError> {
    if let Some(reason) = request.reason.as_deref()
        && !is_bounded_ascii(reason, 96)
    {
        return Err(SessionError::Invalid(
            "workload eligibility sweep reason must be short printable ASCII".to_string(),
        ));
    }
    Ok(())
}

fn validate_status(status: &str) -> Result<(), SessionError> {
    match status {
        "active" | "disabled" | "deprecated" => Ok(()),
        _ => Err(SessionError::Invalid(
            "workload policy status must be active, disabled, or deprecated".to_string(),
        )),
    }
}

fn validate_id(label: &str, value: &str, maximum_len: usize) -> Result<(), SessionError> {
    let valid = !value.trim().is_empty()
        && value.len() <= maximum_len
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(SessionError::Invalid(format!(
            "{label} must be a short ASCII identifier"
        )))
    }
}

fn validate_finite_range(
    label: &str,
    value: Option<f64>,
    minimum: f64,
    maximum: f64,
) -> Result<(), SessionError> {
    if let Some(value) = value
        && (!value.is_finite() || value < minimum || value > maximum)
    {
        return Err(SessionError::Invalid(format!(
            "{label} must be finite and between {minimum} and {maximum}"
        )));
    }
    Ok(())
}

fn validate_age(label: &str, value: Option<u32>) -> Result<(), SessionError> {
    if let Some(value) = value
        && value == 0
    {
        return Err(SessionError::Invalid(format!("{label} must be positive")));
    }
    Ok(())
}

fn is_bounded_ascii(value: &str, maximum_len: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum_len
        && value
            .chars()
            .all(|character| character.is_ascii() && !character.is_control())
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
    use uuid::Uuid;

    fn policy_request() -> UpsertWorkloadPolicyRequest {
        UpsertWorkloadPolicyRequest {
            policy_id: "llm_realtime_api_cuda".to_string(),
            policy_version: "2026.07.0".to_string(),
            workload_type: "llm_realtime_api".to_string(),
            display_name: "LLM realtime CUDA".to_string(),
            description: Some("Short inference workload policy".to_string()),
            requirements: WorkloadPolicyRequirements {
                min_vram_gb: Some(8.0),
                required_backend: Some("cuda".to_string()),
                benchmark_profile_id: Some("llm_realtime_api_small".to_string()),
                benchmark_profile_version: Some("2026.07.0".to_string()),
                benchmark_max_age_seconds: Some(86_400),
                min_tokens_per_second: Some(20.0),
                max_ttft_ms: Some(500.0),
                min_trust_score: Some(60.0),
                max_risk_score: Some(35.0),
                min_reliability_score: Some(50.0),
                min_remote_network_score: Some(50.0),
                required_verification_status: Some("verified".to_string()),
                recent_proof_max_age_seconds: Some(86_400),
                ..Default::default()
            },
            status: Some("active".to_string()),
        }
    }

    fn eligible_candidate(now: DateTime<Utc>) -> WorkloadCandidate {
        WorkloadCandidate {
            provider_id: "provider_1".to_string(),
            provider_status: "available".to_string(),
            device_id: "device_1".to_string(),
            device_status: "active".to_string(),
            policy_id: "llm_realtime_api_cuda".to_string(),
            policy_version: "2026.07.0".to_string(),
            workload_type: "llm_realtime_api".to_string(),
            policy_status: "active".to_string(),
            requirements: policy_request().requirements,
            trust_status: Some("trusted".to_string()),
            trust_score: Some(82.0),
            risk_score: Some(12.0),
            reliability_score: Some(76.0),
            verification_status: Some("verified".to_string()),
            last_verified_at: Some((now - Duration::minutes(5)).to_rfc3339()),
            remote_network_score: Some(88.0),
            regional_reachability: Vec::new(),
            session_status: Some("online".to_string()),
            session_hardware_fingerprint: Some("sha256:fingerprint".to_string()),
            latest_gpu_uuid: Some("GPU-test".to_string()),
            vram_total_mib: Some(16 * 1024),
            benchmark_result_id: Some("benchmark_result_1".to_string()),
            benchmark_profile_id: Some("llm_realtime_api_small".to_string()),
            benchmark_profile_version: Some("2026.07.0".to_string()),
            benchmark_backend: Some("cuda".to_string()),
            benchmark_hardware_fingerprint: Some("sha256:fingerprint".to_string()),
            benchmark_gpu_uuid: Some("GPU-test".to_string()),
            benchmark_completed_at: Some((now - Duration::minutes(3)).to_rfc3339()),
            benchmark_status: Some("succeeded".to_string()),
            benchmark_metrics: Some(BenchmarkResultMetrics {
                tokens_per_second: Some(42.0),
                ttft_ms: Some(180.0),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn policy_validation_rejects_secrets_without_rejecting_token_counts() {
        assert!(validate_workload_policy_request(&policy_request()).is_ok());

        let mut secret = policy_request();
        secret.requirements.allowed_regions = vec!["secret_region".to_string()];
        assert!(validate_workload_policy_request(&secret).is_err());
    }

    #[test]
    fn evaluation_marks_satisfied_policy_as_eligible() {
        let now = Utc::now();
        let evaluation = evaluate_workload_candidate(&eligible_candidate(now), now);
        assert_eq!(evaluation.status, "eligible");
        assert!(
            evaluation
                .reason_codes
                .iter()
                .any(|reason| reason == "policy_requirements_satisfied")
        );
    }

    #[test]
    fn evaluation_marks_missing_benchmark_as_verification_required() {
        let now = Utc::now();
        let mut candidate = eligible_candidate(now);
        candidate.benchmark_result_id = None;
        candidate.benchmark_metrics = None;
        let evaluation = evaluate_workload_candidate(&candidate, now);
        assert_eq!(evaluation.status, "verification_required");
        assert!(
            evaluation
                .reason_codes
                .iter()
                .any(|reason| reason == "benchmark_result_missing")
        );
    }

    #[tokio::test]
    #[ignore]
    async fn persists_policy_and_eligibility_state() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        client
            .execute(
                "INSERT INTO providers (provider_id, user_id, display_name, status, created_at, updated_at) VALUES ($1, NULL, $2, 'available', $3, $3)",
                &[&"provider_1", &"Provider", &now],
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

        let mut request = policy_request();
        request.requirements = WorkloadPolicyRequirements::default();
        let policy = db
            .upsert_workload_policy("req_policy", &request)
            .await
            .unwrap()
            .policy;
        assert_eq!(policy.status, "active");

        let response = db
            .run_workload_eligibility_sweep(
                "req_sweep",
                &RunWorkloadEligibilityRequest {
                    limit: Some(10),
                    force: true,
                    reason: Some("test".to_string()),
                },
            )
            .await
            .unwrap();
        assert_eq!(response.evaluated, 1);
        assert_eq!(response.updated[0].status, "verification_required");
        assert!(
            response.updated[0]
                .reason_codes
                .iter()
                .any(|reason| reason == "trust_state_missing")
        );

        let states = db
            .list_provider_workload_eligibility("req_list", "provider_1")
            .await
            .unwrap();
        assert_eq!(states.states.len(), 1);
        assert_eq!(states.states[0].policy_id, "llm_realtime_api_cuda");
        db.drop_schema_for_test().await.unwrap();
    }
}
