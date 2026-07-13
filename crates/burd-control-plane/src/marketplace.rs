use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::SessionError;
use burd_protocol::{
    BenchmarkResultMetrics, ListMarketplaceListingsResponse, MARKETPLACE_ENGINE_VERSION,
    MARKETPLACE_LISTING_SCHEMA_VERSION, MarketplaceListingRecord, RegionalReachability,
    RunMarketplaceListingSweepRequest, RunMarketplaceListingSweepResponse, hash_canonical,
};
use chrono::Utc;
use tokio_postgres::Row;
use uuid::Uuid;

const MARKETPLACE_SWEEP_LIMIT: u32 = 500;
const MAX_MARKETPLACE_LIST_LIMIT: u32 = 200;

#[derive(Debug, Clone)]
struct MarketplaceCandidate {
    provider_id: String,
    provider_display_name: Option<String>,
    provider_status: String,
    device_id: String,
    device_status: String,
    session_id: Option<String>,
    session_status: Option<String>,
    workload_type: String,
    policy_id: String,
    policy_version: String,
    eligibility_status: String,
    eligibility_reason_codes: Vec<String>,
    trust_score: Option<f64>,
    risk_score: Option<f64>,
    reliability_score: Option<f64>,
    verification_status: Option<String>,
    last_verified_at: Option<String>,
    remote_network_score: Option<f64>,
    effective_network_score: Option<f64>,
    regional_reachability: Vec<RegionalReachability>,
    latest_gpu_uuid: Option<String>,
    vram_total_mib: Option<i64>,
    benchmark_result_id: Option<String>,
    benchmark_profile_id: Option<String>,
    benchmark_profile_version: Option<String>,
    benchmark_status: Option<String>,
    benchmark_completed_at: Option<String>,
    benchmark_gpu_uuid: Option<String>,
    benchmark_metrics: Option<BenchmarkResultMetrics>,
    active_lease_count: u32,
    active_reservation_count: u32,
}

#[derive(Debug, Clone)]
struct ListingEvaluation {
    status: String,
    current_status: String,
    gpu_verified: bool,
    gpu_verification_source: String,
    vram_verified: bool,
    vram_verification_source: String,
    region: Option<String>,
    region_source: String,
    proof_freshness_status: String,
    price_source: String,
    availability_window: serde_json::Value,
    reason_codes: Vec<String>,
}

impl Database {
    pub async fn run_marketplace_listing_sweep(
        &self,
        request_id: &str,
        request: &RunMarketplaceListingSweepRequest,
    ) -> Result<RunMarketplaceListingSweepResponse, SessionError> {
        validate_marketplace_sweep_request(request)?;
        let limit = request
            .limit
            .unwrap_or(MARKETPLACE_SWEEP_LIMIT)
            .min(MARKETPLACE_SWEEP_LIMIT);
        let candidates = self.marketplace_candidates(limit).await?;
        let evaluated = candidates.len() as u32;
        let now = Utc::now().to_rfc3339();
        let mut listings = Vec::with_capacity(candidates.len());
        let mut published = 0_u32;
        let mut skipped = 0_u32;

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        for candidate in candidates {
            let evaluation = evaluate_marketplace_candidate(&candidate);
            if matches!(evaluation.status.as_str(), "published" | "limited") {
                published += 1;
            }
            if evaluation.status == "blocked" {
                skipped += 1;
            }
            let listing_id = format!("listing_{}", Uuid::new_v4());
            let source_hash = hash_canonical(&serde_json::json!({
                "provider_id": candidate.provider_id,
                "device_id": candidate.device_id,
                "workload_type": candidate.workload_type,
                "policy_id": candidate.policy_id,
                "policy_version": candidate.policy_version,
                "eligibility_status": candidate.eligibility_status,
                "trust_score": candidate.trust_score,
                "risk_score": candidate.risk_score,
                "reliability_score": candidate.reliability_score,
                "verification_status": candidate.verification_status,
                "last_verified_at": candidate.last_verified_at,
                "remote_network_score": candidate.remote_network_score,
                "effective_network_score": candidate.effective_network_score,
                "latest_gpu_uuid": candidate.latest_gpu_uuid,
                "vram_total_mib": candidate.vram_total_mib,
                "benchmark_result_id": candidate.benchmark_result_id,
                "benchmark_status": candidate.benchmark_status,
                "active_lease_count": candidate.active_lease_count,
                "active_reservation_count": candidate.active_reservation_count,
            }))
            .map_err(SessionError::Invalid)?;
            let regional_reachability_json =
                serde_json::to_string(&candidate.regional_reachability)
                    .map_err(|error| SessionError::Invalid(error.to_string()))?;
            let benchmark_metrics_json = candidate
                .benchmark_metrics
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|error| SessionError::Invalid(error.to_string()))?;
            let availability_window_json =
                serde_json::to_string(&evaluation.availability_window)
                    .map_err(|error| SessionError::Invalid(error.to_string()))?;
            let reason_codes_json = serde_json::to_string(&evaluation.reason_codes)
                .map_err(|error| SessionError::Invalid(error.to_string()))?;
            let published_at = if matches!(evaluation.status.as_str(), "published" | "limited") {
                Some(now.clone())
            } else {
                None
            };
            let vram_total_mib = candidate.vram_total_mib.map(|value| value.max(0));
            transaction
                .execute(
                    "INSERT INTO marketplace_listings (listing_id, provider_id, provider_display_name, device_id, session_id, schema_version, engine_version, status, current_status, workload_type, policy_id, policy_version, gpu_uuid, gpu_verified, gpu_verification_source, vram_total_mib, vram_verified, vram_verification_source, region, region_source, trust_score, risk_score, reliability_score, verification_status, proof_freshness_status, last_verified_at, remote_network_score, effective_network_score, regional_reachability_json, benchmark_result_id, benchmark_profile_id, benchmark_profile_version, benchmark_status, benchmark_completed_at, benchmark_metrics_json, price_currency, price_per_hour_micros, price_source, availability_window_json, active_lease_count, reason_codes_json, source_hash, published_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34, $35, NULL, NULL, $36, $37, $38, $39, $40, $41, $42) ON CONFLICT (provider_id, device_id, workload_type, policy_id, policy_version) DO UPDATE SET provider_display_name = EXCLUDED.provider_display_name, session_id = EXCLUDED.session_id, schema_version = EXCLUDED.schema_version, engine_version = EXCLUDED.engine_version, status = EXCLUDED.status, current_status = EXCLUDED.current_status, gpu_uuid = EXCLUDED.gpu_uuid, gpu_verified = EXCLUDED.gpu_verified, gpu_verification_source = EXCLUDED.gpu_verification_source, vram_total_mib = EXCLUDED.vram_total_mib, vram_verified = EXCLUDED.vram_verified, vram_verification_source = EXCLUDED.vram_verification_source, region = EXCLUDED.region, region_source = EXCLUDED.region_source, trust_score = EXCLUDED.trust_score, risk_score = EXCLUDED.risk_score, reliability_score = EXCLUDED.reliability_score, verification_status = EXCLUDED.verification_status, proof_freshness_status = EXCLUDED.proof_freshness_status, last_verified_at = EXCLUDED.last_verified_at, remote_network_score = EXCLUDED.remote_network_score, effective_network_score = EXCLUDED.effective_network_score, regional_reachability_json = EXCLUDED.regional_reachability_json, benchmark_result_id = EXCLUDED.benchmark_result_id, benchmark_profile_id = EXCLUDED.benchmark_profile_id, benchmark_profile_version = EXCLUDED.benchmark_profile_version, benchmark_status = EXCLUDED.benchmark_status, benchmark_completed_at = EXCLUDED.benchmark_completed_at, benchmark_metrics_json = EXCLUDED.benchmark_metrics_json, price_currency = EXCLUDED.price_currency, price_per_hour_micros = EXCLUDED.price_per_hour_micros, price_source = EXCLUDED.price_source, availability_window_json = EXCLUDED.availability_window_json, active_lease_count = EXCLUDED.active_lease_count, reason_codes_json = EXCLUDED.reason_codes_json, source_hash = EXCLUDED.source_hash, published_at = EXCLUDED.published_at, updated_at = EXCLUDED.updated_at",
                    &[
                        &listing_id, &candidate.provider_id, &candidate.provider_display_name,
                        &candidate.device_id, &candidate.session_id, &MARKETPLACE_LISTING_SCHEMA_VERSION,
                        &MARKETPLACE_ENGINE_VERSION, &evaluation.status, &evaluation.current_status,
                        &candidate.workload_type, &candidate.policy_id, &candidate.policy_version,
                        &candidate.latest_gpu_uuid, &evaluation.gpu_verified, &evaluation.gpu_verification_source,
                        &vram_total_mib, &evaluation.vram_verified, &evaluation.vram_verification_source,
                        &evaluation.region, &evaluation.region_source, &candidate.trust_score, &candidate.risk_score,
                        &candidate.reliability_score, &candidate.verification_status, &evaluation.proof_freshness_status,
                        &candidate.last_verified_at, &candidate.remote_network_score, &candidate.effective_network_score,
                        &regional_reachability_json, &candidate.benchmark_result_id, &candidate.benchmark_profile_id,
                        &candidate.benchmark_profile_version, &candidate.benchmark_status, &candidate.benchmark_completed_at,
                        &benchmark_metrics_json, &evaluation.price_source, &availability_window_json,
                        &(candidate.active_lease_count as i32), &reason_codes_json, &source_hash, &published_at, &now,
                    ],
                )
                .await?;
            let row = transaction
                .query_one(
                    &format!(
                        "{} WHERE provider_id = $1 AND device_id = $2 AND workload_type = $3 AND policy_id = $4 AND policy_version = $5",
                        marketplace_listing_select_columns()
                    ),
                    &[
                        &candidate.provider_id,
                        &candidate.device_id,
                        &candidate.workload_type,
                        &candidate.policy_id,
                        &candidate.policy_version,
                    ],
                )
                .await?;
            let listing = marketplace_listing_from_row(row)?;
            let audit_metadata = serde_json::json!({
                "provider_id": listing.provider_id,
                "device_id": listing.device_id,
                "workload_type": listing.workload_type,
                "policy_id": listing.policy_id,
                "policy_version": listing.policy_version,
                "status": listing.status,
                "current_status": listing.current_status,
                "gpu_verified": listing.gpu_verified,
                "vram_verified": listing.vram_verified,
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "system",
                    actor_id: None,
                    entity_type: "marketplace_listing",
                    entity_id: &listing.listing_id,
                    event_type: "marketplace_listing.recalculated",
                    idempotency_key: None,
                    summary: "backend marketplace listing recalculated",
                    metadata_json: &audit_metadata,
                },
            )
            .await?;
            listings.push(listing);
        }
        transaction.commit().await?;

        Ok(RunMarketplaceListingSweepResponse {
            request_id: request_id.to_string(),
            evaluated,
            published,
            updated: listings.len() as u32,
            skipped,
            listings,
        })
    }

    pub async fn list_marketplace_listings(
        &self,
        request_id: &str,
        status: Option<&str>,
        workload_type: Option<&str>,
        limit: u32,
    ) -> Result<ListMarketplaceListingsResponse, SessionError> {
        if let Some(status) = status {
            validate_id("status", status, 64)?;
        }
        if let Some(workload_type) = workload_type {
            validate_id("workload_type", workload_type, 96)?;
        }
        let limit = limit.clamp(1, MAX_MARKETPLACE_LIST_LIMIT) as i64;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE (($1::TEXT IS NULL AND status IN ('published', 'limited')) OR status = $1) AND ($2::TEXT IS NULL OR workload_type = $2) ORDER BY trust_score DESC NULLS LAST, reliability_score DESC NULLS LAST, remote_network_score DESC NULLS LAST, updated_at DESC LIMIT $3",
                    marketplace_listing_select_columns()
                ),
                &[&status, &workload_type, &limit],
            )
            .await?;
        let listings = rows
            .into_iter()
            .map(marketplace_listing_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListMarketplaceListingsResponse {
            request_id: request_id.to_string(),
            listings,
        })
    }

    pub async fn list_provider_marketplace_listings(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListMarketplaceListingsResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let limit = limit.clamp(1, MAX_MARKETPLACE_LIST_LIMIT) as i64;
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "{} WHERE provider_id = $1 ORDER BY status, workload_type, updated_at DESC LIMIT $2",
                    marketplace_listing_select_columns()
                ),
                &[&provider_id, &limit],
            )
            .await?;
        let listings = rows
            .into_iter()
            .map(marketplace_listing_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListMarketplaceListingsResponse {
            request_id: request_id.to_string(),
            listings,
        })
    }

    async fn marketplace_candidates(
        &self,
        limit: u32,
    ) -> Result<Vec<MarketplaceCandidate>, SessionError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT pwe.provider_id, p.display_name AS provider_display_name, p.status AS provider_status, pwe.device_id, d.status AS device_status, latest_session.session_id, COALESCE(latest_session.status, pwe.session_status) AS session_status, pwe.workload_type, pwe.policy_id, pwe.policy_version, pwe.status AS eligibility_status, pwe.reason_codes_json AS eligibility_reason_codes_json, pwe.trust_score, pwe.risk_score, pwe.reliability_score, pwe.verification_status, vs.last_verified_at, pwe.remote_network_score, ns.effective_network_score, pwe.regional_reachability_json, pwe.latest_gpu_uuid, pwe.vram_total_mib, pwe.benchmark_result_id, pwe.benchmark_profile_id, pwe.benchmark_profile_version, pwe.benchmark_status, pwe.benchmark_completed_at, br.gpu_uuid AS benchmark_gpu_uuid, br.metrics_json AS benchmark_metrics_json, COALESCE(active_leases.active_lease_count, 0) AS active_lease_count, COALESCE(active_reservations.active_reservation_count, 0) AS active_reservation_count FROM provider_workload_eligibility pwe JOIN providers p ON p.provider_id = pwe.provider_id JOIN devices d ON d.device_id = pwe.device_id AND d.provider_id = pwe.provider_id LEFT JOIN provider_verification_states vs ON vs.provider_id = pwe.provider_id AND vs.device_id = pwe.device_id LEFT JOIN provider_network_states ns ON ns.provider_id = pwe.provider_id AND ns.device_id = pwe.device_id LEFT JOIN LATERAL (SELECT session_id, status, started_at FROM provider_sessions s WHERE s.provider_id = pwe.provider_id AND s.device_id = pwe.device_id ORDER BY started_at DESC LIMIT 1) latest_session ON TRUE LEFT JOIN benchmark_results br ON br.result_id = pwe.benchmark_result_id LEFT JOIN LATERAL (SELECT COUNT(*)::BIGINT AS active_lease_count FROM job_leases jl WHERE jl.provider_id = pwe.provider_id AND jl.device_id = pwe.device_id AND (pwe.latest_gpu_uuid IS NULL OR jl.gpu_uuid = pwe.latest_gpu_uuid) AND jl.status IN ('offered', 'accepted', 'provisioning', 'active')) active_leases ON TRUE LEFT JOIN LATERAL (SELECT COUNT(*)::BIGINT AS active_reservation_count FROM marketplace_reservations mr WHERE mr.provider_id = pwe.provider_id AND mr.device_id = pwe.device_id AND mr.workload_type = pwe.workload_type AND (pwe.latest_gpu_uuid IS NULL OR mr.gpu_uuid = pwe.latest_gpu_uuid) AND mr.status = 'reserved') active_reservations ON TRUE ORDER BY pwe.updated_at DESC, pwe.provider_id, pwe.device_id, pwe.workload_type LIMIT $1",
                &[&(limit as i64)],
            )
            .await?;
        rows.into_iter()
            .map(marketplace_candidate_from_row)
            .collect()
    }
}
fn evaluate_marketplace_candidate(candidate: &MarketplaceCandidate) -> ListingEvaluation {
    let mut reason_codes = candidate.eligibility_reason_codes.clone();
    let provider_or_device_blocked = matches!(
        candidate.provider_status.as_str(),
        "blocked" | "quarantined"
    ) || candidate.device_status != "active";
    let benchmark_succeeded = candidate.benchmark_result_id.is_some()
        && candidate.benchmark_status.as_deref() == Some("succeeded");
    let benchmark_gpu_matches = match (
        candidate.latest_gpu_uuid.as_deref(),
        candidate.benchmark_gpu_uuid.as_deref(),
    ) {
        (Some(latest), Some(benchmark)) => latest == benchmark,
        (Some(_), None) => benchmark_succeeded,
        _ => false,
    };
    let proof_verified = candidate.verification_status.as_deref() == Some("verified");
    let gpu_verified =
        candidate.latest_gpu_uuid.is_some() && proof_verified && benchmark_gpu_matches;
    let gpu_verification_source = if gpu_verified {
        "backend_proof_and_benchmark".to_string()
    } else if candidate.latest_gpu_uuid.is_some() {
        "backend_observed_unverified".to_string()
    } else {
        "unobserved".to_string()
    };
    if !gpu_verified {
        reason_codes.push("marketplace_gpu_not_backend_verified".to_string());
    }

    let vram_verified = candidate.vram_total_mib.is_some() && gpu_verified;
    let vram_verification_source = if vram_verified {
        "backend_telemetry_bound_to_verified_gpu".to_string()
    } else if candidate.vram_total_mib.is_some() {
        "backend_observed_unverified".to_string()
    } else {
        "unobserved".to_string()
    };
    if !vram_verified {
        reason_codes.push("marketplace_vram_not_backend_verified".to_string());
    }

    let (region, region_source) = observed_region(&candidate.regional_reachability);
    if region.is_none() {
        reason_codes.push("marketplace_region_unverified".to_string());
    }
    let proof_freshness_status = match (
        candidate.verification_status.as_deref(),
        candidate.last_verified_at.as_deref(),
    ) {
        (Some("verified"), Some(_)) => "freshness_backend_timestamp_present".to_string(),
        (Some("verified"), None) => "verified_without_timestamp".to_string(),
        (Some(status), _) => format!("not_verified_{status}"),
        (None, _) => "verification_missing".to_string(),
    };
    let price_source = "not_configured_bn16".to_string();
    reason_codes.push("marketplace_price_not_configured".to_string());

    let status = if provider_or_device_blocked {
        reason_codes.push("provider_or_device_blocked".to_string());
        "blocked".to_string()
    } else if !gpu_verified || !vram_verified {
        "verification_required".to_string()
    } else {
        match candidate.eligibility_status.as_str() {
            "eligible" => "published".to_string(),
            "limited" => "limited".to_string(),
            "temporarily_unavailable" => "temporarily_unavailable".to_string(),
            "verification_required" => "verification_required".to_string(),
            "blocked" | "ineligible" => "blocked".to_string(),
            _ => "verification_required".to_string(),
        }
    };
    let current_status = current_marketplace_status(candidate, &status);
    let availability_window = serde_json::json!({
        "mode": "session_bound_now",
        "source": "remote_session_and_active_leases",
        "reservations_enabled": false,
        "session_status": candidate.session_status,
        "active_lease_count": candidate.active_lease_count,
        "active_reservation_count": candidate.active_reservation_count,
    });
    reason_codes.sort();
    reason_codes.dedup();

    ListingEvaluation {
        status,
        current_status,
        gpu_verified,
        gpu_verification_source,
        vram_verified,
        vram_verification_source,
        region,
        region_source,
        proof_freshness_status,
        price_source,
        availability_window,
        reason_codes,
    }
}

fn current_marketplace_status(candidate: &MarketplaceCandidate, listing_status: &str) -> String {
    if listing_status == "blocked" {
        return "blocked".to_string();
    }
    if candidate.active_lease_count > 0 || candidate.active_reservation_count > 0 {
        return "reserved".to_string();
    }
    match candidate.session_status.as_deref() {
        Some("online") if matches!(listing_status, "published" | "limited") => {
            "available".to_string()
        }
        Some("degraded") if matches!(listing_status, "published" | "limited") => {
            "degraded".to_string()
        }
        Some("online" | "degraded") => listing_status.to_string(),
        Some("offline" | "expired" | "revoked" | "pending_connection") => "offline".to_string(),
        _ => "offline".to_string(),
    }
}

fn observed_region(reachability: &[RegionalReachability]) -> (Option<String>, String) {
    for region in reachability {
        if region.status == "unreachable" {
            continue;
        }
        if let Some(approximate) = region.approximate_region.as_ref() {
            return (
                Some(approximate.clone()),
                "regional_probe_approximate".to_string(),
            );
        }
        return (
            Some(region.probe_region.clone()),
            "regional_probe".to_string(),
        );
    }
    (None, "unobserved".to_string())
}
fn marketplace_candidate_from_row(row: Row) -> Result<MarketplaceCandidate, SessionError> {
    let reason_codes_json: String = row.get("eligibility_reason_codes_json");
    let regional_reachability_json: String = row.get("regional_reachability_json");
    let metrics_json: Option<String> = row.get("benchmark_metrics_json");
    let active_lease_count: i64 = row.get("active_lease_count");
    let active_reservation_count: i64 = row.get("active_reservation_count");
    Ok(MarketplaceCandidate {
        provider_id: row.get("provider_id"),
        provider_display_name: row.get("provider_display_name"),
        provider_status: row.get("provider_status"),
        device_id: row.get("device_id"),
        device_status: row.get("device_status"),
        session_id: row.get("session_id"),
        session_status: row.get("session_status"),
        workload_type: row.get("workload_type"),
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
        eligibility_status: row.get("eligibility_status"),
        eligibility_reason_codes: serde_json::from_str(&reason_codes_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        trust_score: row.get("trust_score"),
        risk_score: row.get("risk_score"),
        reliability_score: row.get("reliability_score"),
        verification_status: row.get("verification_status"),
        last_verified_at: row.get("last_verified_at"),
        remote_network_score: row.get("remote_network_score"),
        effective_network_score: row.get("effective_network_score"),
        regional_reachability: serde_json::from_str(&regional_reachability_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        latest_gpu_uuid: row.get("latest_gpu_uuid"),
        vram_total_mib: row.get("vram_total_mib"),
        benchmark_result_id: row.get("benchmark_result_id"),
        benchmark_profile_id: row.get("benchmark_profile_id"),
        benchmark_profile_version: row.get("benchmark_profile_version"),
        benchmark_status: row.get("benchmark_status"),
        benchmark_completed_at: row.get("benchmark_completed_at"),
        benchmark_gpu_uuid: row.get("benchmark_gpu_uuid"),
        benchmark_metrics: metrics_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        active_lease_count: active_lease_count.max(0) as u32,
        active_reservation_count: active_reservation_count.max(0) as u32,
    })
}

fn marketplace_listing_from_row(row: Row) -> Result<MarketplaceListingRecord, SessionError> {
    let regional_reachability_json: String = row.get("regional_reachability_json");
    let benchmark_metrics_json: Option<String> = row.get("benchmark_metrics_json");
    let availability_window_json: String = row.get("availability_window_json");
    let reason_codes_json: String = row.get("reason_codes_json");
    Ok(MarketplaceListingRecord {
        listing_id: row.get("listing_id"),
        provider_id: row.get("provider_id"),
        provider_display_name: row.get("provider_display_name"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        schema_version: row.get("schema_version"),
        engine_version: row.get("engine_version"),
        status: row.get("status"),
        current_status: row.get("current_status"),
        workload_type: row.get("workload_type"),
        policy_id: row.get("policy_id"),
        policy_version: row.get("policy_version"),
        gpu_uuid: row.get("gpu_uuid"),
        gpu_verified: row.get("gpu_verified"),
        gpu_verification_source: row.get("gpu_verification_source"),
        vram_total_mib: row
            .get::<_, Option<i64>>("vram_total_mib")
            .map(from_i64)
            .transpose()?,
        vram_verified: row.get("vram_verified"),
        vram_verification_source: row.get("vram_verification_source"),
        region: row.get("region"),
        region_source: row.get("region_source"),
        trust_score: row.get("trust_score"),
        risk_score: row.get("risk_score"),
        reliability_score: row.get("reliability_score"),
        verification_status: row.get("verification_status"),
        proof_freshness_status: row.get("proof_freshness_status"),
        last_verified_at: row.get("last_verified_at"),
        remote_network_score: row.get("remote_network_score"),
        effective_network_score: row.get("effective_network_score"),
        regional_reachability: serde_json::from_str(&regional_reachability_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        benchmark_result_id: row.get("benchmark_result_id"),
        benchmark_profile_id: row.get("benchmark_profile_id"),
        benchmark_profile_version: row.get("benchmark_profile_version"),
        benchmark_status: row.get("benchmark_status"),
        benchmark_completed_at: row.get("benchmark_completed_at"),
        benchmark_metrics: benchmark_metrics_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        price_currency: row.get("price_currency"),
        price_per_hour_micros: row
            .get::<_, Option<i64>>("price_per_hour_micros")
            .map(from_i64)
            .transpose()?,
        price_source: row.get("price_source"),
        availability_window: serde_json::from_str(&availability_window_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        active_lease_count: from_i32(row.get("active_lease_count"))?,
        reason_codes: serde_json::from_str(&reason_codes_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        source_hash: row.get("source_hash"),
        published_at: row.get("published_at"),
        updated_at: row.get("updated_at"),
    })
}

fn marketplace_listing_select_columns() -> &'static str {
    "SELECT listing_id, provider_id, provider_display_name, device_id, session_id, schema_version, engine_version, status, current_status, workload_type, policy_id, policy_version, gpu_uuid, gpu_verified, gpu_verification_source, vram_total_mib, vram_verified, vram_verification_source, region, region_source, trust_score, risk_score, reliability_score, verification_status, proof_freshness_status, last_verified_at, remote_network_score, effective_network_score, regional_reachability_json, benchmark_result_id, benchmark_profile_id, benchmark_profile_version, benchmark_status, benchmark_completed_at, benchmark_metrics_json, price_currency, price_per_hour_micros, price_source, availability_window_json, active_lease_count, reason_codes_json, source_hash, published_at, updated_at FROM marketplace_listings"
}

fn validate_marketplace_sweep_request(
    request: &RunMarketplaceListingSweepRequest,
) -> Result<(), SessionError> {
    if let Some(limit) = request.limit
        && limit == 0
    {
        return Err(SessionError::Invalid(
            "limit must be greater than zero".to_string(),
        ));
    }
    if let Some(reason) = request.reason.as_deref()
        && !is_bounded_ascii(reason, 160)
    {
        return Err(SessionError::Invalid(
            "marketplace sweep reason must be printable ASCII".to_string(),
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

fn is_bounded_ascii(value: &str, maximum_len: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum_len
        && value
            .chars()
            .all(|character| character.is_ascii_graphic() || character == ' ')
}

fn from_i64(value: i64) -> Result<u64, SessionError> {
    u64::try_from(value)
        .map_err(|_| SessionError::Database(DbError::new("negative marketplace quantity")))
}

fn from_i32(value: i32) -> Result<u32, SessionError> {
    u32::try_from(value)
        .map_err(|_| SessionError::Database(DbError::new("negative marketplace count")))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn verified_candidate() -> MarketplaceCandidate {
        MarketplaceCandidate {
            provider_id: "provider_1".to_string(),
            provider_display_name: Some("Provider".to_string()),
            provider_status: "available".to_string(),
            device_id: "device_1".to_string(),
            device_status: "active".to_string(),
            session_id: Some("session_1".to_string()),
            session_status: Some("online".to_string()),
            workload_type: "llm_realtime_api".to_string(),
            policy_id: "llm_realtime_api_cuda".to_string(),
            policy_version: "2026.07.0".to_string(),
            eligibility_status: "eligible".to_string(),
            eligibility_reason_codes: vec!["policy_requirements_satisfied".to_string()],
            trust_score: Some(92.0),
            risk_score: Some(4.0),
            reliability_score: Some(98.0),
            verification_status: Some("verified".to_string()),
            last_verified_at: Some("2026-07-13T00:00:00Z".to_string()),
            remote_network_score: Some(91.0),
            effective_network_score: Some(90.0),
            regional_reachability: vec![RegionalReachability {
                probe_region: "us-east".to_string(),
                status: "reachable".to_string(),
                remote_network_score: 91.0,
                sample_count: 3,
                observed_at: "2026-07-13T00:00:00Z".to_string(),
                approximate_region: Some("us-east".to_string()),
                control_rtt_ms: Some(20.0),
                packet_loss_percent: Some(0.0),
            }],
            latest_gpu_uuid: Some("GPU-test".to_string()),
            vram_total_mib: Some(24_576),
            benchmark_result_id: Some("benchmark_result_1".to_string()),
            benchmark_profile_id: Some("profile_llm".to_string()),
            benchmark_profile_version: Some("v1".to_string()),
            benchmark_status: Some("succeeded".to_string()),
            benchmark_completed_at: Some("2026-07-13T00:00:00Z".to_string()),
            benchmark_gpu_uuid: Some("GPU-test".to_string()),
            benchmark_metrics: Some(BenchmarkResultMetrics {
                tokens_per_second: Some(128.0),
                ..BenchmarkResultMetrics::default()
            }),
            active_lease_count: 0,
            active_reservation_count: 0,
        }
    }

    #[test]
    fn verified_eligible_candidate_publishes_available_listing() {
        let candidate = verified_candidate();
        let evaluation = evaluate_marketplace_candidate(&candidate);
        assert_eq!(evaluation.status, "published");
        assert_eq!(evaluation.current_status, "available");
        assert!(evaluation.gpu_verified);
        assert!(evaluation.vram_verified);
        assert_eq!(evaluation.region.as_deref(), Some("us-east"));
    }

    #[test]
    fn observed_but_unverified_candidate_does_not_publish_verified_gpu() {
        let mut candidate = verified_candidate();
        candidate.verification_status = Some("verification_due".to_string());
        let evaluation = evaluate_marketplace_candidate(&candidate);
        assert_eq!(evaluation.status, "verification_required");
        assert!(!evaluation.gpu_verified);
        assert!(!evaluation.vram_verified);
        assert!(
            evaluation
                .reason_codes
                .iter()
                .any(|reason| reason == "marketplace_gpu_not_backend_verified")
        );
    }

    #[test]
    fn active_lease_marks_listing_reserved() {
        let mut candidate = verified_candidate();
        candidate.active_lease_count = 1;
        let evaluation = evaluate_marketplace_candidate(&candidate);
        assert_eq!(evaluation.status, "published");
        assert_eq!(evaluation.current_status, "reserved");
    }

    #[test]
    fn marketplace_sweep_request_rejects_empty_reason() {
        let error = validate_marketplace_sweep_request(&RunMarketplaceListingSweepRequest {
            limit: Some(1),
            force: true,
            reason: Some("".to_string()),
        })
        .unwrap_err();
        assert!(error.to_string().contains("reason"));
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn persists_marketplace_listing_from_backend_states() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let client = db.connect().await.unwrap();
        client
            .batch_execute(
                r#"
                INSERT INTO providers (provider_id, user_id, display_name, status, created_at, updated_at)
                VALUES ('provider_1', NULL, 'Marketplace Provider', 'available', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at)
                VALUES ('device_1', 'provider_1', 'machine_1', 'active', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint)
                VALUES ('session_1', 'provider_1', 'device_1', 'online', 0, '2026-07-13T00:00:00Z', '2026-07-13T01:00:00Z', 'fp_1');
                INSERT INTO provider_verification_states (provider_id, device_id, status, policy_version, reason, risk_score, success_count, failure_count, retry_budget_remaining, last_verified_at, created_at, updated_at)
                VALUES ('provider_1', 'device_1', 'verified', 'test', NULL, 0, 1, 0, 2, '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO provider_network_states (provider_id, device_id, local_network_score, remote_network_score, regional_reachability_json, effective_network_score, sample_count, last_observed_at, updated_at)
                VALUES ('provider_1', 'device_1', NULL, 91, '[{"probe_region":"us-east","status":"reachable","remote_network_score":91.0,"sample_count":1,"observed_at":"2026-07-13T00:00:00Z","approximate_region":"us-east"}]', 91, 1, '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO provider_trust_states (provider_id, device_id, status, policy_version, trust_score, risk_score, reliability_score, verification_status, remote_network_score, evidence_count, successful_challenge_count, failed_challenge_count, session_status, latest_gpu_uuid, hardware_fingerprint, reason_codes_json, created_at, updated_at)
                VALUES ('provider_1', 'device_1', 'trusted', 'test', 92, 4, 98, 'verified', 91, 1, 1, 0, 'online', 'GPU-test', 'fp_1', '[]', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO workload_policies (policy_id, policy_version, schema_version, workload_type, display_name, description, requirements_json, status, created_at, updated_at)
                VALUES ('llm_realtime_api_cuda', '2026.07.0', 'burd-workload-policy-v2', 'llm_realtime_api', 'LLM realtime', NULL, '{}', 'active', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO benchmark_profiles (profile_id, profile_version, schema_version, workload_type, display_name, description, image_digest, model_hash, artifact_hash, required_backend, min_vram_gb, parameters_json, warmup_seconds, duration_seconds, sample_count, thresholds_json, status, created_at, updated_at)
                VALUES ('profile_llm', 'v1', 'burd-benchmark-profile-v2', 'llm_realtime_api', 'Profile', NULL, 'sha256:image', NULL, NULL, 'cuda', 8, '{}', 1, 1, 1, '{}', 'active', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at)
                VALUES ('key_1', 'provider_1', 'device_1', 'public', 'ed25519', 'active', '2026-07-13T00:00:00Z');
                INSERT INTO benchmark_results (result_id, provider_id, device_id, session_id, run_id, profile_id, profile_version, schema_version, workload_type, backend, hardware_fingerprint, gpu_uuid, image_digest, model_hash, artifact_hash, parameters_json, warmup_seconds, duration_seconds, sample_count, started_at, completed_at, server_received_at, driver_version, cuda_driver_version, cuda_runtime_version, metrics_json, telemetry_window_hash, result_hash, public_key_id, signature, canonicalization_version, status, verification_json, warnings_json)
                VALUES ('benchmark_result_1', 'provider_1', 'device_1', 'session_1', 'run_1', 'profile_llm', 'v1', 'burd-benchmark-result-v1', 'llm_realtime_api', 'cuda', 'fp_1', 'GPU-test', 'sha256:image', NULL, NULL, '{}', 1, 1, 1, '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z', 'driver', NULL, NULL, '{"tokens_per_second":128.0}', NULL, 'hash_1', 'key_1', 'sig', 'burd-json-c14n-v1', 'succeeded', '{}', '[]');
                INSERT INTO provider_workload_eligibility (provider_id, device_id, workload_type, policy_id, policy_version, schema_version, engine_version, status, reason_codes_json, trust_score, risk_score, reliability_score, verification_status, remote_network_score, benchmark_result_id, benchmark_profile_id, benchmark_profile_version, benchmark_backend, benchmark_completed_at, benchmark_status, session_status, latest_gpu_uuid, vram_total_mib, hardware_fingerprint, regional_reachability_json, evaluated_at, updated_at)
                VALUES ('provider_1', 'device_1', 'llm_realtime_api', 'llm_realtime_api_cuda', '2026.07.0', 'burd-workload-eligibility-v2', 'burd-workload-policy-engine-v1', 'eligible', '[]', 92, 4, 98, 'verified', 91, 'benchmark_result_1', 'profile_llm', 'v1', 'cuda', '2026-07-13T00:00:00Z', 'succeeded', 'online', 'GPU-test', 24576, 'fp_1', '[{"probe_region":"us-east","status":"reachable","remote_network_score":91.0,"sample_count":1,"observed_at":"2026-07-13T00:00:00Z","approximate_region":"us-east"}]', '2026-07-13T00:00:00Z', '2026-07-13T00:00:00Z');
                "#,
            )
            .await
            .unwrap();

        let response = db
            .run_marketplace_listing_sweep(
                "req_market",
                &RunMarketplaceListingSweepRequest {
                    limit: Some(10),
                    force: true,
                    reason: Some("test".to_string()),
                },
            )
            .await
            .unwrap();
        assert_eq!(response.evaluated, 1);
        assert_eq!(response.published, 1);
        assert_eq!(response.listings[0].status, "published");
        assert!(response.listings[0].gpu_verified);
        assert!(response.listings[0].vram_verified);

        let public_list = db
            .list_marketplace_listings("req_list", None, Some("llm_realtime_api"), 10)
            .await
            .unwrap();
        assert_eq!(public_list.listings.len(), 1);
        db.drop_schema_for_test().await.unwrap();
    }
}
