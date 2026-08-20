use crate::remote_session::{SessionError, SessionError::Conflict};
use burd_protocol::{
    AGENT_CONTROL_PROTOCOL_POLICY_VERSION, AGENT_CONTROL_PROTOCOL_VERSION,
    AGENT_RUNTIME_CONTRACT_VERSION, JOB_ARTIFACT_UPLOAD_VERSION, JOB_DATA_PLANE_GRANT_VERSION,
    JOB_EXECUTION_CONTROL_SCHEMA_VERSION, PROVIDER_JOB_EXECUTION_SCHEMA_VERSION,
    RemoteSessionProtocolNegotiation, RemoteSessionProtocolNegotiationStatus,
};
use chrono::Utc;
use semver::{Version, VersionReq};
use std::collections::BTreeSet;
use tokio_postgres::Transaction;

pub const MAX_PROTOCOL_VERSIONS: usize = 8;
pub const MAX_PROTOCOL_CAPABILITIES: usize = 32;
pub const MAX_PROTOCOL_IDENTIFIER_LEN: usize = 128;
pub const MINIMUM_AGENT_VERSION: &str = "0.1.0";
pub const SUPPORTED_AGENT_VERSION_RANGE: &str = ">=0.1.0, <0.2.0";

const REQUIRED_CAPABILITIES: &[&str] = &[
    PROVIDER_JOB_EXECUTION_SCHEMA_VERSION,
    JOB_EXECUTION_CONTROL_SCHEMA_VERSION,
    JOB_DATA_PLANE_GRANT_VERSION,
    JOB_ARTIFACT_UPLOAD_VERSION,
    AGENT_RUNTIME_CONTRACT_VERSION,
];

#[derive(Debug, Clone)]
pub struct AgentProtocolPolicy {
    pub policy_version: &'static str,
    pub supported_agent_version_range: &'static str,
    pub minimum_agent_version: &'static str,
    pub supported_protocol_versions: &'static [&'static str],
    pub allowed_capabilities: &'static [&'static str],
    pub required_capabilities: &'static [&'static str],
}

pub const fn current_agent_protocol_policy() -> AgentProtocolPolicy {
    AgentProtocolPolicy {
        policy_version: AGENT_CONTROL_PROTOCOL_POLICY_VERSION,
        supported_agent_version_range: SUPPORTED_AGENT_VERSION_RANGE,
        minimum_agent_version: MINIMUM_AGENT_VERSION,
        supported_protocol_versions: &[AGENT_CONTROL_PROTOCOL_VERSION],
        allowed_capabilities: REQUIRED_CAPABILITIES,
        required_capabilities: REQUIRED_CAPABILITIES,
    }
}

pub fn negotiate_agent_protocol(
    agent_version: &str,
    declared_versions: &[String],
    declared_capabilities: &[String],
    policy: &AgentProtocolPolicy,
) -> Result<(RemoteSessionProtocolNegotiation, Vec<String>, Vec<String>), SessionError> {
    let versions = normalize_identifiers(
        "supported_protocol_versions",
        declared_versions,
        MAX_PROTOCOL_VERSIONS,
    )?;
    let capabilities = normalize_identifiers(
        "supported_capabilities",
        declared_capabilities,
        MAX_PROTOCOL_CAPABILITIES,
    )?;
    let required = sorted(policy.required_capabilities);
    let now = Utc::now().to_rfc3339();

    if versions.is_empty() && capabilities.is_empty() {
        return Ok((
            RemoteSessionProtocolNegotiation {
                status: RemoteSessionProtocolNegotiationStatus::LegacyUnnegotiated,
                selected_protocol_version: None,
                minimum_agent_version: policy.minimum_agent_version.to_string(),
                required_capabilities: required,
                accepted_capabilities: Vec::new(),
                policy_version: policy.policy_version.to_string(),
                reason_codes: vec!["legacy_request_missing_protocol_declaration".to_string()],
                negotiated_at: Some(now),
            },
            versions,
            capabilities,
        ));
    }

    let parsed_version = Version::parse(agent_version).map_err(|_| {
        SessionError::Invalid("agent_version must be a valid semantic version".to_string())
    })?;
    let supported_range = VersionReq::parse(policy.supported_agent_version_range)
        .map_err(|error| SessionError::Database(crate::db::DbError::new(error.to_string())))?;
    if !supported_range.matches(&parsed_version) {
        return Ok((
            rejected(
                RemoteSessionProtocolNegotiationStatus::UpgradeRequired,
                policy,
                required,
                "agent_version_outside_supported_range",
                now,
            ),
            versions,
            capabilities,
        ));
    }

    let selected = policy
        .supported_protocol_versions
        .iter()
        .find(|candidate| versions.iter().any(|version| version == **candidate))
        .map(|value| (*value).to_string());
    let Some(selected) = selected else {
        return Ok((
            rejected(
                RemoteSessionProtocolNegotiationStatus::IncompatibleProtocol,
                policy,
                required,
                "no_common_protocol_version",
                now,
            ),
            versions,
            capabilities,
        ));
    };

    let allowed = policy
        .allowed_capabilities
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let accepted = capabilities
        .iter()
        .filter(|capability| allowed.contains(capability.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let accepted_set = accepted.iter().map(String::as_str).collect::<BTreeSet<_>>();
    if policy
        .required_capabilities
        .iter()
        .any(|required| !accepted_set.contains(required))
    {
        return Ok((
            RemoteSessionProtocolNegotiation {
                status: RemoteSessionProtocolNegotiationStatus::MissingCapabilities,
                selected_protocol_version: None,
                minimum_agent_version: policy.minimum_agent_version.to_string(),
                required_capabilities: required,
                accepted_capabilities: accepted,
                policy_version: policy.policy_version.to_string(),
                reason_codes: vec!["required_protocol_capability_missing".to_string()],
                negotiated_at: Some(now),
            },
            versions,
            capabilities,
        ));
    }

    Ok((
        RemoteSessionProtocolNegotiation {
            status: RemoteSessionProtocolNegotiationStatus::Accepted,
            selected_protocol_version: Some(selected),
            minimum_agent_version: policy.minimum_agent_version.to_string(),
            required_capabilities: required,
            accepted_capabilities: accepted,
            policy_version: policy.policy_version.to_string(),
            reason_codes: vec!["protocol_negotiation_accepted".to_string()],
            negotiated_at: Some(now),
        },
        versions,
        capabilities,
    ))
}

pub async fn assert_current_compute_protocol_negotiation(
    transaction: &Transaction<'_>,
    session_id: &str,
) -> Result<(), SessionError> {
    let policy = current_agent_protocol_policy();
    let row = transaction
        .query_opt(
            "SELECT protocol_negotiation_status, negotiated_protocol_version, protocol_policy_version, accepted_protocol_capabilities_json FROM provider_sessions WHERE session_id = $1 FOR SHARE",
            &[&session_id],
        )
        .await?
        .ok_or_else(|| SessionError::NotFound("remote session not found".to_string()))?;
    let status: String = row.get("protocol_negotiation_status");
    let version: Option<String> = row.get("negotiated_protocol_version");
    let policy_version: Option<String> = row.get("protocol_policy_version");
    let accepted_json: String = row.get("accepted_protocol_capabilities_json");
    let accepted: Vec<String> = serde_json::from_str(&accepted_json)
        .map_err(|_| Conflict("session protocol negotiation is invalid".to_string()))?;
    let accepted = accepted.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let authorized = status == RemoteSessionProtocolNegotiationStatus::Accepted.as_str()
        && version
            .as_deref()
            .is_some_and(|value| policy.supported_protocol_versions.contains(&value))
        && policy_version.as_deref() == Some(policy.policy_version)
        && policy
            .required_capabilities
            .iter()
            .all(|required| accepted.contains(required));
    if authorized {
        Ok(())
    } else {
        Err(Conflict(
            "session has no current accepted protocol negotiation".to_string(),
        ))
    }
}

fn rejected(
    status: RemoteSessionProtocolNegotiationStatus,
    policy: &AgentProtocolPolicy,
    required_capabilities: Vec<String>,
    reason: &str,
    negotiated_at: String,
) -> RemoteSessionProtocolNegotiation {
    RemoteSessionProtocolNegotiation {
        status,
        selected_protocol_version: None,
        minimum_agent_version: policy.minimum_agent_version.to_string(),
        required_capabilities,
        accepted_capabilities: Vec::new(),
        policy_version: policy.policy_version.to_string(),
        reason_codes: vec![reason.to_string()],
        negotiated_at: Some(negotiated_at),
    }
}

fn normalize_identifiers(
    label: &str,
    values: &[String],
    maximum: usize,
) -> Result<Vec<String>, SessionError> {
    if values.len() > maximum {
        return Err(SessionError::Invalid(format!(
            "{label} exceeds maximum items"
        )));
    }
    let mut normalized = BTreeSet::new();
    for value in values {
        if value.is_empty()
            || value.len() > MAX_PROTOCOL_IDENTIFIER_LEN
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(SessionError::Invalid(format!(
                "{label} contains an invalid identifier"
            )));
        }
        normalized.insert(value.clone());
    }
    Ok(normalized.into_iter().collect())
}

fn sorted(values: &[&str]) -> Vec<String> {
    values
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declarations() -> (Vec<String>, Vec<String>) {
        (
            vec![AGENT_CONTROL_PROTOCOL_VERSION.to_string()],
            REQUIRED_CAPABILITIES
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        )
    }

    #[test]
    fn compatible_agent_is_accepted_and_unknown_capability_is_not() {
        let (versions, mut capabilities) = declarations();
        capabilities.push("unknown-future-capability-v9".to_string());
        let (result, _, declared) = negotiate_agent_protocol(
            "0.1.0",
            &versions,
            &capabilities,
            &current_agent_protocol_policy(),
        )
        .unwrap();
        assert_eq!(
            result.status,
            RemoteSessionProtocolNegotiationStatus::Accepted
        );
        assert!(
            !result
                .accepted_capabilities
                .contains(&capabilities.last().unwrap().clone())
        );
        assert!(declared.contains(capabilities.last().unwrap()));
    }

    #[test]
    fn legacy_old_future_protocol_and_missing_capability_fail_closed() {
        let policy = current_agent_protocol_policy();
        assert_eq!(
            negotiate_agent_protocol("not-semver", &[], &[], &policy)
                .unwrap()
                .0
                .status,
            RemoteSessionProtocolNegotiationStatus::LegacyUnnegotiated
        );
        let (versions, capabilities) = declarations();
        for version in ["0.0.9", "0.2.0"] {
            assert_eq!(
                negotiate_agent_protocol(version, &versions, &capabilities, &policy)
                    .unwrap()
                    .0
                    .status,
                RemoteSessionProtocolNegotiationStatus::UpgradeRequired
            );
        }
        assert!(negotiate_agent_protocol("invalid", &versions, &capabilities, &policy).is_err());
        assert_eq!(
            negotiate_agent_protocol("0.1.0", &["unknown-v2".to_string()], &capabilities, &policy)
                .unwrap()
                .0
                .status,
            RemoteSessionProtocolNegotiationStatus::IncompatibleProtocol
        );
        assert_eq!(
            negotiate_agent_protocol("0.1.0", &versions, &capabilities[..4], &policy)
                .unwrap()
                .0
                .status,
            RemoteSessionProtocolNegotiationStatus::MissingCapabilities
        );
    }
}
