use crate::db::{Database, DbError, NewAuditEvent, insert_audit_event};
use crate::remote_session::SessionError;
use burd_protocol::{
    ListNetworkProbeObservationsResponse, ListProviderNetworkStatesResponse,
    NETWORK_PROBE_SCHEMA_VERSION, NetworkProbeObservationRecord, ProviderNetworkState,
    RegionalReachability, SubmitNetworkProbeObservationRequest,
    SubmitNetworkProbeObservationResponse,
};
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::{Row, Transaction};
use uuid::Uuid;

const MAX_RECENT_OBSERVATIONS: i64 = 32;

impl Database {
    pub async fn submit_network_probe_observation(
        &self,
        request_id: &str,
        request: &SubmitNetworkProbeObservationRequest,
    ) -> Result<SubmitNetworkProbeObservationResponse, SessionError> {
        validate_probe_observation(request)?;
        let (remote_network_score, warnings) = score_probe_observation(request);
        let status = network_status(remote_network_score).to_string();
        let warnings_json = serde_json::to_string(&warnings)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let metadata = normalized_metadata(&request.metadata)?;
        let metadata_json = serde_json::to_string(&metadata)
            .map_err(|error| SessionError::Invalid(error.to_string()))?;
        let server_received_at = Utc::now().to_rfc3339();
        let observation_id = format!("network_probe_{}", Uuid::new_v4());

        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        assert_session_accepts_probe(&transaction, request).await?;

        let inserted = transaction
            .execute(
                "INSERT INTO network_probe_observations (observation_id, provider_id, device_id, session_id, probe_id, probe_region, schema_version, observed_at, server_received_at, sample_count, control_rtt_ms, jitter_ms, packet_loss_percent, reconnect_count, upload_mbps, download_mbps, artifact_throughput_mbps, stability_score, approximate_region, path_consistency, remote_network_score, status, warnings_json, metadata_json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24) ON CONFLICT (session_id, probe_id, observed_at) DO NOTHING",
                &[
                    &observation_id,
                    &request.provider_id,
                    &request.device_id,
                    &request.session_id,
                    &request.probe_id,
                    &request.probe_region,
                    &NETWORK_PROBE_SCHEMA_VERSION,
                    &request.observed_at,
                    &server_received_at,
                    &(request.sample_count as i32),
                    &request.control_rtt_ms,
                    &request.jitter_ms,
                    &request.packet_loss_percent,
                    &request.reconnect_count.map(|value| value as i32),
                    &request.upload_mbps,
                    &request.download_mbps,
                    &request.artifact_throughput_mbps,
                    &request.stability_score,
                    &request.approximate_region,
                    &request.path_consistency,
                    &remote_network_score,
                    &status,
                    &warnings_json,
                    &metadata_json,
                ],
            )
            .await?
            == 1;

        let observation_row = if inserted {
            transaction
                .query_one(
                    observation_select_sql("WHERE observation_id = $1"),
                    &[&observation_id],
                )
                .await?
        } else {
            transaction
                .query_one(
                    observation_select_sql(
                        "WHERE session_id = $1 AND probe_id = $2 AND observed_at = $3",
                    ),
                    &[&request.session_id, &request.probe_id, &request.observed_at],
                )
                .await?
        };
        let observation = observation_from_row(observation_row)?;
        let network_state = refresh_provider_network_state(
            &transaction,
            &request.provider_id,
            &request.device_id,
            &Utc::now().to_rfc3339(),
        )
        .await?;

        if inserted {
            let audit_metadata = serde_json::json!({
                "observation_id": observation.observation_id,
                "session_id": request.session_id,
                "probe_id": request.probe_id,
                "probe_region": request.probe_region,
                "remote_network_score": observation.remote_network_score,
                "status": observation.status,
            })
            .to_string();
            insert_audit_event(
                &transaction,
                NewAuditEvent {
                    request_id,
                    actor_type: "network_probe",
                    actor_id: Some(request.probe_id.clone()),
                    entity_type: "provider_network_state",
                    entity_id: &request.device_id,
                    event_type: "network_probe.observed",
                    idempotency_key: None,
                    summary: "regional network probe observation accepted",
                    metadata_json: &audit_metadata,
                },
            )
            .await?;
        }

        transaction.commit().await?;
        Ok(SubmitNetworkProbeObservationResponse {
            request_id: request_id.to_string(),
            duplicate: !inserted,
            observation,
            network_state,
        })
    }

    pub async fn list_network_probe_observations(
        &self,
        request_id: &str,
        provider_id: &str,
        limit: u32,
    ) -> Result<ListNetworkProbeObservationsResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let limit = limit.clamp(1, 200) as i64;
        let client = self.connect().await?;
        let rows = client
            .query(
                observation_select_sql(
                    "WHERE provider_id = $1 ORDER BY observed_at DESC, server_received_at DESC LIMIT $2",
                ),
                &[&provider_id, &limit],
            )
            .await?;
        let observations = rows
            .into_iter()
            .map(observation_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListNetworkProbeObservationsResponse {
            request_id: request_id.to_string(),
            observations,
        })
    }

    pub async fn list_provider_network_states(
        &self,
        request_id: &str,
        provider_id: &str,
    ) -> Result<ListProviderNetworkStatesResponse, SessionError> {
        validate_id("provider_id", provider_id, 128)?;
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT provider_id, device_id, local_network_score, remote_network_score, regional_reachability_json, effective_network_score, sample_count, last_observed_at, updated_at FROM provider_network_states WHERE provider_id = $1 ORDER BY effective_network_score DESC NULLS LAST, updated_at DESC",
                &[&provider_id],
            )
            .await?;
        let states = rows
            .into_iter()
            .map(network_state_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListProviderNetworkStatesResponse {
            request_id: request_id.to_string(),
            states,
        })
    }
}

async fn assert_session_accepts_probe(
    transaction: &Transaction<'_>,
    request: &SubmitNetworkProbeObservationRequest,
) -> Result<(), SessionError> {
    let row = transaction
        .query_opt(
            "SELECT s.status, s.expires_at, p.status AS provider_status, d.status AS device_status FROM provider_sessions s JOIN providers p ON p.provider_id = s.provider_id JOIN devices d ON d.device_id = s.device_id WHERE s.session_id = $1 AND s.provider_id = $2 AND s.device_id = $3 AND d.provider_id = $2 FOR UPDATE",
            &[&request.session_id, &request.provider_id, &request.device_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
    let provider_status: String = row.get("provider_status");
    let device_status: String = row.get("device_status");
    let session_status: String = row.get("status");
    let expires_at: String = row.get("expires_at");
    if session_status == "revoked" {
        return Err(SessionError::Revoked);
    }
    if session_status == "expired" || timestamp_expired(&expires_at)? {
        return Err(SessionError::Expired);
    }
    if matches!(provider_status.as_str(), "blocked" | "quarantined") || device_status != "active" {
        return Err(SessionError::Revoked);
    }
    if !matches!(session_status.as_str(), "online" | "degraded" | "offline") {
        return Err(SessionError::Conflict(
            "network probes require an observed remote session".to_string(),
        ));
    }
    Ok(())
}

async fn refresh_provider_network_state(
    transaction: &Transaction<'_>,
    provider_id: &str,
    device_id: &str,
    updated_at: &str,
) -> Result<ProviderNetworkState, SessionError> {
    let local_network_score = transaction
        .query_opt(
            "SELECT local_network_score FROM provider_network_states WHERE provider_id = $1 AND device_id = $2",
            &[&provider_id, &device_id],
        )
        .await?
        .and_then(|row| row.get("local_network_score"));
    let recent_rows = transaction
        .query(
            "SELECT remote_network_score, observed_at FROM network_probe_observations WHERE provider_id = $1 AND device_id = $2 ORDER BY observed_at DESC, server_received_at DESC LIMIT $3",
            &[&provider_id, &device_id, &MAX_RECENT_OBSERVATIONS],
        )
        .await?;
    if recent_rows.is_empty() {
        return Err(SessionError::NotFound(
            "network probe observation not found".to_string(),
        ));
    }
    let total_score = recent_rows
        .iter()
        .map(|row| row.get::<_, f64>("remote_network_score"))
        .sum::<f64>();
    let remote_network_score = round_score(total_score / recent_rows.len() as f64);
    let last_observed_at = recent_rows
        .first()
        .map(|row| row.get::<_, String>("observed_at"));
    let regional_reachability =
        latest_regional_reachability(transaction, provider_id, device_id).await?;
    let effective_network_score = Some(match local_network_score {
        Some(local) => round_score((remote_network_score * 0.7) + (local * 0.3)),
        None => remote_network_score,
    });
    let reachability_json = serde_json::to_string(&regional_reachability)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    transaction
        .execute(
            "INSERT INTO provider_network_states (provider_id, device_id, local_network_score, remote_network_score, regional_reachability_json, effective_network_score, sample_count, last_observed_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (provider_id, device_id) DO UPDATE SET local_network_score = COALESCE(provider_network_states.local_network_score, EXCLUDED.local_network_score), remote_network_score = EXCLUDED.remote_network_score, regional_reachability_json = EXCLUDED.regional_reachability_json, effective_network_score = EXCLUDED.effective_network_score, sample_count = EXCLUDED.sample_count, last_observed_at = EXCLUDED.last_observed_at, updated_at = EXCLUDED.updated_at",
            &[
                &provider_id,
                &device_id,
                &local_network_score,
                &Some(remote_network_score),
                &reachability_json,
                &effective_network_score,
                &(recent_rows.len() as i32),
                &last_observed_at,
                &updated_at,
            ],
        )
        .await?;
    Ok(ProviderNetworkState {
        provider_id: provider_id.to_string(),
        device_id: device_id.to_string(),
        local_network_score,
        remote_network_score: Some(remote_network_score),
        regional_reachability,
        effective_network_score,
        sample_count: recent_rows.len() as u32,
        last_observed_at,
        updated_at: updated_at.to_string(),
    })
}

async fn latest_regional_reachability(
    transaction: &Transaction<'_>,
    provider_id: &str,
    device_id: &str,
) -> Result<Vec<RegionalReachability>, SessionError> {
    let rows = transaction
        .query(
            "SELECT DISTINCT ON (probe_region) probe_region, status, remote_network_score, sample_count, observed_at, approximate_region, control_rtt_ms, packet_loss_percent FROM network_probe_observations WHERE provider_id = $1 AND device_id = $2 ORDER BY probe_region, observed_at DESC, server_received_at DESC",
            &[&provider_id, &device_id],
        )
        .await?;
    rows.into_iter()
        .map(regional_reachability_from_row)
        .collect()
}

fn validate_probe_observation(
    request: &SubmitNetworkProbeObservationRequest,
) -> Result<(), SessionError> {
    validate_id("provider_id", &request.provider_id, 128)?;
    validate_id("device_id", &request.device_id, 128)?;
    validate_id("session_id", &request.session_id, 128)?;
    validate_id("probe_id", &request.probe_id, 128)?;
    validate_id("probe_region", &request.probe_region, 64)?;
    if request.sample_count == 0 || request.sample_count > 10_000 {
        return Err(SessionError::Invalid(
            "network probe sample_count must be between 1 and 10000".to_string(),
        ));
    }
    let observed_at = DateTime::parse_from_rfc3339(&request.observed_at)
        .map_err(|error| SessionError::Invalid(format!("observed_at must be RFC3339: {error}")))?;
    if observed_at.with_timezone(&Utc) > Utc::now() + Duration::minutes(5) {
        return Err(SessionError::Invalid(
            "network probe observed_at is too far in the future".to_string(),
        ));
    }
    let mut has_metric = false;
    for (label, value, maximum) in [
        ("control_rtt_ms", request.control_rtt_ms, 120_000.0),
        ("jitter_ms", request.jitter_ms, 120_000.0),
        ("packet_loss_percent", request.packet_loss_percent, 100.0),
        ("upload_mbps", request.upload_mbps, 1_000_000.0),
        ("download_mbps", request.download_mbps, 1_000_000.0),
        (
            "artifact_throughput_mbps",
            request.artifact_throughput_mbps,
            1_000_000.0,
        ),
        ("stability_score", request.stability_score, 100.0),
        ("path_consistency", request.path_consistency, 100.0),
    ] {
        if let Some(value) = value {
            has_metric = true;
            if !value.is_finite() || value < 0.0 || value > maximum {
                return Err(SessionError::Invalid(format!(
                    "{label} must be finite and between 0 and {maximum}"
                )));
            }
        }
    }
    if request.reconnect_count.is_some_and(|value| value > 1_000) {
        return Err(SessionError::Invalid(
            "reconnect_count must be between 0 and 1000".to_string(),
        ));
    }
    has_metric |= request.reconnect_count.is_some();
    if !has_metric {
        return Err(SessionError::Invalid(
            "network probe observation must contain at least one measurement".to_string(),
        ));
    }
    if request
        .approximate_region
        .as_deref()
        .is_some_and(|region| region.trim().is_empty() || region.len() > 64)
    {
        return Err(SessionError::Invalid(
            "approximate_region is invalid".to_string(),
        ));
    }
    let metadata = normalized_metadata(&request.metadata)?;
    let metadata_json = serde_json::to_string(&metadata)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    if metadata_json.len() > 8 * 1024 || contains_secret_field(&metadata) {
        return Err(SessionError::Invalid(
            "network probe metadata must be small and redacted".to_string(),
        ));
    }
    Ok(())
}

fn timestamp_expired(raw: &str) -> Result<bool, SessionError> {
    let timestamp = DateTime::parse_from_rfc3339(raw)
        .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?;
    Ok(timestamp <= Utc::now())
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
        Err(SessionError::Invalid(format!("{label} is invalid")))
    }
}

fn normalized_metadata(value: &serde_json::Value) -> Result<serde_json::Value, SessionError> {
    if value.is_null() {
        return Ok(serde_json::json!({}));
    }
    if !matches!(value, serde_json::Value::Object(_)) {
        return Err(SessionError::Invalid(
            "network probe metadata must be a JSON object".to_string(),
        ));
    }
    Ok(value.clone())
}

fn score_probe_observation(request: &SubmitNetworkProbeObservationRequest) -> (f64, Vec<String>) {
    let mut score = 100.0;
    let mut warnings = Vec::new();

    if let Some(rtt) = request.control_rtt_ms {
        if rtt > 50.0 {
            score -= ((rtt - 50.0) / 10.0).min(35.0);
        }
        if rtt > 250.0 {
            warnings.push("high_control_rtt".to_string());
        }
    } else {
        warnings.push("missing_control_rtt".to_string());
    }
    if let Some(jitter) = request.jitter_ms {
        if jitter > 10.0 {
            score -= ((jitter - 10.0) / 5.0).min(20.0);
        }
        if jitter > 50.0 {
            warnings.push("high_jitter".to_string());
        }
    }
    if let Some(loss) = request.packet_loss_percent {
        score -= (loss * 4.0).min(40.0);
        if loss > 1.0 {
            warnings.push("packet_loss".to_string());
        }
    }
    if let Some(reconnects) = request.reconnect_count {
        score -= (f64::from(reconnects) * 8.0).min(24.0);
        if reconnects > 0 {
            warnings.push("reconnects_observed".to_string());
        }
    }

    let throughput_floor = [
        request.upload_mbps,
        request.download_mbps,
        request.artifact_throughput_mbps,
    ]
    .into_iter()
    .flatten()
    .fold(None, |minimum: Option<f64>, value| {
        Some(minimum.map_or(value, |current| current.min(value)))
    });
    if let Some(throughput) = throughput_floor {
        if throughput < 25.0 {
            score -= (25.0 - throughput).min(20.0);
            warnings.push("low_throughput".to_string());
        }
    } else {
        warnings.push("missing_throughput".to_string());
    }
    if let Some(stability) = request.stability_score {
        score = (score * 0.7) + (stability * 0.3);
        if stability < 80.0 {
            warnings.push("low_stability".to_string());
        }
    }
    if let Some(path_consistency) = request.path_consistency {
        score = (score * 0.85) + (path_consistency * 0.15);
        if path_consistency < 80.0 {
            warnings.push("path_inconsistent".to_string());
        }
    }

    (round_score(score.clamp(0.0, 100.0)), warnings)
}

fn network_status(score: f64) -> &'static str {
    if score >= 80.0 {
        "reachable"
    } else if score >= 50.0 {
        "degraded"
    } else {
        "unreachable"
    }
}

fn round_score(score: f64) -> f64 {
    (score * 100.0).round() / 100.0
}

fn observation_select_sql(where_clause: &str) -> &'static str {
    match where_clause {
        "WHERE observation_id = $1" => {
            "SELECT observation_id, provider_id, device_id, session_id, probe_id, probe_region, schema_version, observed_at, server_received_at, sample_count, control_rtt_ms, jitter_ms, packet_loss_percent, reconnect_count, upload_mbps, download_mbps, artifact_throughput_mbps, stability_score, approximate_region, path_consistency, remote_network_score, status, warnings_json, metadata_json FROM network_probe_observations WHERE observation_id = $1"
        }
        "WHERE session_id = $1 AND probe_id = $2 AND observed_at = $3" => {
            "SELECT observation_id, provider_id, device_id, session_id, probe_id, probe_region, schema_version, observed_at, server_received_at, sample_count, control_rtt_ms, jitter_ms, packet_loss_percent, reconnect_count, upload_mbps, download_mbps, artifact_throughput_mbps, stability_score, approximate_region, path_consistency, remote_network_score, status, warnings_json, metadata_json FROM network_probe_observations WHERE session_id = $1 AND probe_id = $2 AND observed_at = $3"
        }
        "WHERE provider_id = $1 ORDER BY observed_at DESC, server_received_at DESC LIMIT $2" => {
            "SELECT observation_id, provider_id, device_id, session_id, probe_id, probe_region, schema_version, observed_at, server_received_at, sample_count, control_rtt_ms, jitter_ms, packet_loss_percent, reconnect_count, upload_mbps, download_mbps, artifact_throughput_mbps, stability_score, approximate_region, path_consistency, remote_network_score, status, warnings_json, metadata_json FROM network_probe_observations WHERE provider_id = $1 ORDER BY observed_at DESC, server_received_at DESC LIMIT $2"
        }
        _ => unreachable!("unsupported network probe select clause"),
    }
}

fn observation_from_row(row: Row) -> Result<NetworkProbeObservationRecord, SessionError> {
    let warnings_json: String = row.get("warnings_json");
    let metadata_json: String = row.get("metadata_json");
    Ok(NetworkProbeObservationRecord {
        observation_id: row.get("observation_id"),
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        session_id: row.get("session_id"),
        probe_id: row.get("probe_id"),
        probe_region: row.get("probe_region"),
        schema_version: row.get("schema_version"),
        observed_at: row.get("observed_at"),
        server_received_at: row.get("server_received_at"),
        sample_count: row.get::<_, i32>("sample_count").max(0) as u32,
        control_rtt_ms: row.get("control_rtt_ms"),
        jitter_ms: row.get("jitter_ms"),
        packet_loss_percent: row.get("packet_loss_percent"),
        reconnect_count: row
            .get::<_, Option<i32>>("reconnect_count")
            .map(|value| value.max(0) as u32),
        upload_mbps: row.get("upload_mbps"),
        download_mbps: row.get("download_mbps"),
        artifact_throughput_mbps: row.get("artifact_throughput_mbps"),
        stability_score: row.get("stability_score"),
        approximate_region: row.get("approximate_region"),
        path_consistency: row.get("path_consistency"),
        remote_network_score: row.get("remote_network_score"),
        status: row.get("status"),
        warnings: serde_json::from_str(&warnings_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        metadata: serde_json::from_str(&metadata_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
    })
}

fn network_state_from_row(row: Row) -> Result<ProviderNetworkState, SessionError> {
    let reachability_json: String = row.get("regional_reachability_json");
    Ok(ProviderNetworkState {
        provider_id: row.get("provider_id"),
        device_id: row.get("device_id"),
        local_network_score: row.get("local_network_score"),
        remote_network_score: row.get("remote_network_score"),
        regional_reachability: serde_json::from_str(&reachability_json)
            .map_err(|error| SessionError::Database(DbError::new(error.to_string())))?,
        effective_network_score: row.get("effective_network_score"),
        sample_count: row.get::<_, i32>("sample_count").max(0) as u32,
        last_observed_at: row.get("last_observed_at"),
        updated_at: row.get("updated_at"),
    })
}

fn regional_reachability_from_row(row: Row) -> Result<RegionalReachability, SessionError> {
    Ok(RegionalReachability {
        probe_region: row.get("probe_region"),
        status: row.get("status"),
        remote_network_score: row.get("remote_network_score"),
        sample_count: row.get::<_, i32>("sample_count").max(0) as u32,
        observed_at: row.get("observed_at"),
        approximate_region: row.get("approximate_region"),
        control_rtt_ms: row.get("control_rtt_ms"),
        packet_loss_percent: row.get("packet_loss_percent"),
    })
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
    [
        "password",
        "secret",
        "token",
        "private_key",
        "authorization",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_request() -> SubmitNetworkProbeObservationRequest {
        SubmitNetworkProbeObservationRequest {
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            probe_id: "probe_sao_paulo_1".to_string(),
            probe_region: "sa-east-1".to_string(),
            observed_at: Utc::now().to_rfc3339(),
            sample_count: 12,
            control_rtt_ms: Some(24.0),
            jitter_ms: Some(3.0),
            packet_loss_percent: Some(0.0),
            reconnect_count: Some(0),
            upload_mbps: Some(120.0),
            download_mbps: Some(180.0),
            artifact_throughput_mbps: Some(90.0),
            stability_score: Some(96.0),
            approximate_region: Some("BR-SP".to_string()),
            path_consistency: Some(98.0),
            metadata: serde_json::json!({"collector": "regional-probe"}),
        }
    }

    #[test]
    fn good_probe_scores_as_reachable() {
        let request = probe_request();
        let (score, warnings) = score_probe_observation(&request);
        assert!(score >= 95.0, "score={score}");
        assert_eq!(network_status(score), "reachable");
        assert!(warnings.is_empty());
    }

    #[test]
    fn lossy_probe_scores_as_unreachable() {
        let mut request = probe_request();
        request.control_rtt_ms = Some(700.0);
        request.jitter_ms = Some(120.0);
        request.packet_loss_percent = Some(12.0);
        request.reconnect_count = Some(4);
        request.upload_mbps = Some(2.0);
        request.stability_score = Some(30.0);
        request.path_consistency = Some(40.0);
        let (score, warnings) = score_probe_observation(&request);
        assert!(score < 50.0, "score={score}");
        assert_eq!(network_status(score), "unreachable");
        assert!(warnings.contains(&"packet_loss".to_string()));
    }

    #[test]
    fn validation_rejects_provider_claimed_secret_metadata() {
        let mut request = probe_request();
        request.metadata = serde_json::json!({"token": "leak"});
        assert!(validate_probe_observation(&request).is_err());
    }

    #[tokio::test]
    #[ignore]
    async fn persists_probe_observation_and_network_state() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_test_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        db.migrate().await.unwrap();

        let client = db.connect().await.unwrap();
        let now = Utc::now().to_rfc3339();
        let expires_at = (Utc::now() + Duration::minutes(30)).to_rfc3339();
        client
            .execute(
                "INSERT INTO providers (provider_id, user_id, display_name, status, created_at, updated_at) VALUES ($1, NULL, $2, 'available', $3, $3)",
                &[&"provider_1", &"Probe Provider", &now],
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
                "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at) VALUES ($1, $2, $3, 'online', 0, $4, $5)",
                &[&"session_1", &"provider_1", &"device_1", &now, &expires_at],
            )
            .await
            .unwrap();

        let request = probe_request();
        let response = db
            .submit_network_probe_observation("req_probe", &request)
            .await
            .unwrap();
        assert!(!response.duplicate);
        assert_eq!(response.observation.status, "reachable");
        assert!(response.network_state.remote_network_score.unwrap() >= 95.0);
        assert_eq!(response.network_state.regional_reachability.len(), 1);

        let replay = db
            .submit_network_probe_observation("req_probe_replay", &request)
            .await
            .unwrap();
        assert!(replay.duplicate);
        assert_eq!(replay.network_state.sample_count, 1);

        db.drop_schema_for_test().await.unwrap();
    }
}
