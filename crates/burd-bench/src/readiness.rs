use crate::history::{BenchmarkHistoryList, load_history_list};
use crate::provider::build_provider_details;
use crate::raw::{RawData, build_raw_data_from_provider};
use crate::report::{load_latest_signed_report, verify_signed_report_at};
use crate::verification::ProviderVerification;
use burd_protocol::{
    AgentConfig, AgentStatePaths, ApiTokenStatus, ChallengeRunOutput, EvidenceFreshness,
    PrivateKeyFile, ProviderSession, ProviderSessionStatus, SIGNED_REPORT_TTL_SECONDS,
    SignedReport, agent_state_paths, evidence_freshness_at, load_identity,
    load_latest_challenge_output, load_private_key, load_provider_session, show_api_token_status,
    verify_challenge_response,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const IDENTITY_WEIGHT: u8 = 15;
const SIGNED_REPORT_WEIGHT: u8 = 20;
const CHALLENGE_WEIGHT: u8 = 15;
const PROVIDER_VERIFICATION_WEIGHT: u8 = 20;
const HISTORY_WEIGHT: u8 = 10;
const API_TOKEN_WEIGHT: u8 = 10;
const RAW_REDACTION_WEIGHT: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReadinessStatus {
    Uninitialized,
    NotVerified,
    Partial,
    ReadyLocally,
    Failed,
}

impl ProviderReadinessStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uninitialized => "uninitialized",
            Self::NotVerified => "not_verified",
            Self::Partial => "partial",
            Self::ReadyLocally => "ready_locally",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCheckStatus {
    Passed,
    Warning,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessEvidenceStatus {
    Missing,
    Invalid,
    Expired,
    Valid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessEvidenceSummary {
    pub signed_report: ReadinessEvidenceStatus,
    pub challenge: ReadinessEvidenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_report_freshness: Option<EvidenceFreshness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_freshness: Option<EvidenceFreshness>,
}

impl ReadinessCheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Warning => "warning",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessCheck {
    pub id: String,
    pub label: String,
    pub status: ReadinessCheckStatus,
    pub score: u8,
    pub max_score: u8,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderReadiness {
    pub state: AgentStatePaths,
    pub status: ProviderReadinessStatus,
    pub readiness_score: u8,
    pub readiness_level: String,
    pub evidence: ReadinessEvidenceSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ProviderSession>,
    pub checks: Vec<ReadinessCheck>,
    pub warnings: Vec<String>,
    pub recommendations: Vec<String>,
}

pub fn build_provider_readiness(agent_version: &str, host_uri: &str) -> ProviderReadiness {
    let identity = load_identity();
    let private_key = identity
        .as_ref()
        .map_err(Clone::clone)
        .and_then(load_private_key);
    let signed_report = load_latest_signed_report();
    let history = load_history_list();
    let challenge = load_latest_challenge_output();
    let api_token = show_api_token_status();

    let (verification, raw) = if identity.is_ok() {
        let provider = build_provider_details(agent_version, host_uri);
        let verification = provider.verification.clone();
        let raw = build_raw_data_from_provider(&provider, &verification);
        (Some(verification), Some(raw))
    } else {
        (None, None)
    };

    evaluate_provider_readiness(ProviderReadinessInputs {
        identity,
        private_key,
        signed_report,
        verification,
        history,
        challenge,
        api_token,
        raw,
        now: Utc::now(),
    })
}

pub(crate) struct ProviderReadinessInputs {
    pub identity: Result<AgentConfig, String>,
    pub private_key: Result<PrivateKeyFile, String>,
    pub signed_report: Result<SignedReport, String>,
    pub verification: Option<ProviderVerification>,
    pub history: Result<BenchmarkHistoryList, String>,
    pub challenge: Result<ChallengeRunOutput, String>,
    pub api_token: Result<ApiTokenStatus, String>,
    pub raw: Option<RawData>,
    pub now: DateTime<Utc>,
}

pub(crate) fn evaluate_provider_readiness(inputs: ProviderReadinessInputs) -> ProviderReadiness {
    let identity_missing = inputs
        .identity
        .as_ref()
        .err()
        .is_some_and(|error| error.contains("not found"));
    let mut critical_failure = inputs.identity.is_err() && !identity_missing;
    let mut warnings = Vec::new();
    let mut recommendations = Vec::new();
    let mut checks = Vec::new();
    let state = agent_state_paths();
    let session = load_provider_session().ok().flatten();
    let signed_report_freshness = inputs.signed_report.as_ref().ok().and_then(|report| {
        evidence_freshness_at(&report.signed_at, SIGNED_REPORT_TTL_SECONDS, inputs.now).ok()
    });

    match (&inputs.identity, &inputs.private_key) {
        (Ok(_), Ok(_)) => checks.push(passed(
            "identity",
            "Identity",
            IDENTITY_WEIGHT,
            "Provider identity and private signing key are available.",
        )),
        (Err(_), _) => {
            checks.push(failed(
                "identity",
                "Identity",
                IDENTITY_WEIGHT,
                if identity_missing {
                    "Provider identity is not initialized."
                } else {
                    "Provider identity cannot be loaded."
                },
            ));
            warnings.push(if identity_missing {
                "Provider identity is not initialized.".to_string()
            } else {
                "Provider identity cannot be loaded.".to_string()
            });
            recommendations.push("Run `burd-agent identity init`.".to_string());
        }
        (Ok(_), Err(_)) => {
            critical_failure = true;
            checks.push(failed(
                "identity",
                "Identity",
                IDENTITY_WEIGHT,
                "Provider identity exists, but its private signing key is unavailable.",
            ));
            warnings.push("Provider private signing key is unavailable.".to_string());
            recommendations.push(
                "Restore the configured private key or rotate it with `burd-agent identity rotate-key --confirm`."
                    .to_string(),
            );
        }
    }

    let (signed_report_valid, signed_report_evidence_status) = match &inputs.signed_report {
        Ok(report) => {
            let result = verify_signed_report_at(report, inputs.now);
            if result.signature_valid && result.errors.is_empty() {
                if result
                    .evidence
                    .as_ref()
                    .is_some_and(|evidence| evidence.is_expired)
                {
                    checks.push(warning(
                        "signed_report",
                        "Signed report",
                        SIGNED_REPORT_WEIGHT,
                        "Latest signed report is valid but expired.",
                    ));
                    warnings.push("Latest signed report is expired.".to_string());
                    recommendations.push(
                        "Renew the signed report with `burd-agent report --signed --json`."
                            .to_string(),
                    );
                    (false, ReadinessEvidenceStatus::Expired)
                } else {
                    checks.push(passed(
                        "signed_report",
                        "Signed report",
                        SIGNED_REPORT_WEIGHT,
                        "Latest signed report is valid and unexpired locally.",
                    ));
                    (true, ReadinessEvidenceStatus::Valid)
                }
            } else {
                critical_failure = true;
                checks.push(failed(
                    "signed_report",
                    "Signed report",
                    SIGNED_REPORT_WEIGHT,
                    "Latest signed report failed local verification.",
                ));
                warnings.push("Latest signed report failed local verification.".to_string());
                recommendations.push(
                    "Generate a new signed report with `burd-agent report --signed --json`."
                        .to_string(),
                );
                (false, ReadinessEvidenceStatus::Invalid)
            }
        }
        Err(error) => {
            let malformed = !error.contains("failed to read");
            if malformed {
                critical_failure = true;
            }
            checks.push(if malformed {
                failed(
                    "signed_report",
                    "Signed report",
                    SIGNED_REPORT_WEIGHT,
                    "Latest signed report cannot be parsed.",
                )
            } else {
                warning(
                    "signed_report",
                    "Signed report",
                    SIGNED_REPORT_WEIGHT,
                    "No valid signed report is available.",
                )
            });
            warnings.push("No valid signed report is available.".to_string());
            recommendations.push(
                "Generate a signed report with `burd-agent report --signed --json`.".to_string(),
            );
            (
                false,
                if malformed {
                    ReadinessEvidenceStatus::Invalid
                } else {
                    ReadinessEvidenceStatus::Missing
                },
            )
        }
    };

    let challenge_verification = inputs
        .challenge
        .as_ref()
        .ok()
        .map(|output| verify_challenge_response(&output.challenge, &output.response));
    let challenge_evidence_status = if challenge_verification.as_ref().is_some_and(|verification| {
        verification.valid && verification.signature_valid && !verification.expired
    }) {
        checks.push(passed(
            "challenge",
            "Challenge",
            CHALLENGE_WEIGHT,
            "Latest local challenge evidence is valid and unexpired.",
        ));
        ReadinessEvidenceStatus::Valid
    } else {
        let invalid_evidence = challenge_verification.is_some();
        let expired_evidence = challenge_verification
            .as_ref()
            .is_some_and(|verification| verification.expired);
        checks.push(warning(
            "challenge",
            "Challenge",
            CHALLENGE_WEIGHT,
            if expired_evidence {
                "Latest local challenge evidence is expired."
            } else if invalid_evidence {
                "Latest local challenge evidence is invalid."
            } else {
                "No locally verified challenge evidence is available."
            },
        ));
        warnings.push(if expired_evidence {
            "Provider challenge evidence is expired.".to_string()
        } else if invalid_evidence {
            "Provider challenge evidence is invalid.".to_string()
        } else {
            "Provider challenge is pending.".to_string()
        });
        recommendations.push(
            "Run `burd-agent challenge run-local --json` to create locally verified challenge evidence."
                .to_string(),
        );
        if expired_evidence {
            ReadinessEvidenceStatus::Expired
        } else if invalid_evidence {
            ReadinessEvidenceStatus::Invalid
        } else {
            ReadinessEvidenceStatus::Missing
        }
    };

    let provider_verified = match &inputs.verification {
        Some(verification)
            if verification.hardware_verified
                && verification.benchmark_verified
                && verification.signature_verified
                && verification.audit_status == "self_verified"
                && verification.warnings.is_empty()
                && verification.failed_checks.is_empty() =>
        {
            checks.push(passed(
                "provider_verification",
                "Provider verification",
                PROVIDER_VERIFICATION_WEIGHT,
                "Provider hardware, benchmark, and signature are self-verified.",
            ));
            true
        }
        Some(verification) => {
            let verification_failed = verification.fraud_risk_level == "high"
                || verification
                    .failed_checks
                    .iter()
                    .any(|check| check == "report_signature_invalid");
            if verification_failed {
                critical_failure = true;
                checks.push(failed(
                    "provider_verification",
                    "Provider verification",
                    PROVIDER_VERIFICATION_WEIGHT,
                    "Provider verification contains a critical failure.",
                ));
            } else {
                checks.push(warning(
                    "provider_verification",
                    "Provider verification",
                    PROVIDER_VERIFICATION_WEIGHT,
                    "Provider is not fully self-verified.",
                ));
            }
            warnings.extend(verification.warnings.iter().cloned());
            warnings.extend(
                verification
                    .failed_checks
                    .iter()
                    .map(|check| format!("Provider verification failed check: {check}.")),
            );
            recommendations.push(
                "Resolve provider verification warnings and generate a valid signed report."
                    .to_string(),
            );
            false
        }
        None => {
            checks.push(warning(
                "provider_verification",
                "Provider verification",
                PROVIDER_VERIFICATION_WEIGHT,
                "Provider verification is unavailable until identity initialization.",
            ));
            false
        }
    };

    if let Some(session_info) = &session {
        match session_info.status {
            ProviderSessionStatus::Active
                if session_info.online_locally && !session_info.is_expired =>
            {
                checks.push(passed(
                    "session",
                    "Session",
                    0,
                    "A local provider session is active.",
                ));
            }
            ProviderSessionStatus::Expired => {
                checks.push(warning(
                    "session",
                    "Session",
                    0,
                    "A local provider session has expired.",
                ));
                warnings.push("Local provider session has expired.".to_string());
            }
            ProviderSessionStatus::Invalidated => {
                checks.push(warning(
                    "session",
                    "Session",
                    0,
                    "A local provider session is invalidated.",
                ));
                warnings.push("Local provider session is invalidated.".to_string());
            }
            ProviderSessionStatus::Stopped => {
                checks.push(warning(
                    "session",
                    "Session",
                    0,
                    "A local provider session has been stopped.",
                ));
            }
            ProviderSessionStatus::Failed => {
                checks.push(warning(
                    "session",
                    "Session",
                    0,
                    "A local provider session failed to start.",
                ));
                warnings.push("Local provider session failed.".to_string());
            }
            ProviderSessionStatus::Inactive => {}
            ProviderSessionStatus::Active => {}
        }
    }

    match &inputs.history {
        Ok(history) if history.entries_total > 0 => checks.push(passed(
            "history",
            "History",
            HISTORY_WEIGHT,
            "Benchmark history contains at least one persisted entry.",
        )),
        Ok(_) => {
            checks.push(warning(
                "history",
                "History",
                HISTORY_WEIGHT,
                "Benchmark history is empty.",
            ));
            warnings.push("Benchmark history is empty.".to_string());
            recommendations.push(
                "Generate a report with `burd-agent report --run-all --signed --json` to persist benchmark history."
                    .to_string(),
            );
        }
        Err(_) => {
            critical_failure = true;
            checks.push(failed(
                "history",
                "History",
                HISTORY_WEIGHT,
                "Benchmark history cannot be loaded.",
            ));
            warnings.push("Benchmark history cannot be loaded.".to_string());
            recommendations.push("Repair or clear the local benchmark history file.".to_string());
        }
    }

    match &inputs.api_token {
        Ok(status) if status.api_auth_enabled && status.token_configured => checks.push(passed(
            "api_token",
            "API token",
            API_TOKEN_WEIGHT,
            "Local API authentication is enabled with a configured token.",
        )),
        Ok(_) => {
            checks.push(warning(
                "api_token",
                "API token",
                API_TOKEN_WEIGHT,
                "Local API authentication token is not enabled.",
            ));
            warnings.push("Local API authentication token is not enabled.".to_string());
            recommendations
                .push("Create a token with `burd-agent api-token create --json`.".to_string());
        }
        Err(_) => {
            checks.push(warning(
                "api_token",
                "API token",
                API_TOKEN_WEIGHT,
                "API token status is unavailable.",
            ));
        }
    }

    match &inputs.raw {
        Some(raw) if raw_redaction_is_safe(raw) => checks.push(passed(
            "raw_redaction",
            "Raw redaction",
            RAW_REDACTION_WEIGHT,
            "Raw provider data declares and applies required secret redaction.",
        )),
        Some(_) => {
            critical_failure = true;
            checks.push(failed(
                "raw_redaction",
                "Raw redaction",
                RAW_REDACTION_WEIGHT,
                "Raw provider data does not satisfy the redaction contract.",
            ));
            warnings.push("Raw provider data failed the redaction contract.".to_string());
            recommendations
                .push("Do not expose raw provider data until redaction is restored.".to_string());
        }
        None => checks.push(warning(
            "raw_redaction",
            "Raw redaction",
            RAW_REDACTION_WEIGHT,
            "Raw redaction cannot be evaluated until identity initialization.",
        )),
    }

    deduplicate(&mut warnings);
    deduplicate(&mut recommendations);
    let readiness_score = checks.iter().map(|check| check.score).sum();
    let all_passed = checks
        .iter()
        .all(|check| check.status == ReadinessCheckStatus::Passed);
    let status = if identity_missing {
        ProviderReadinessStatus::Uninitialized
    } else if critical_failure {
        ProviderReadinessStatus::Failed
    } else if all_passed {
        ProviderReadinessStatus::ReadyLocally
    } else if !signed_report_valid && !provider_verified {
        ProviderReadinessStatus::NotVerified
    } else {
        ProviderReadinessStatus::Partial
    };
    let readiness_level = match status {
        ProviderReadinessStatus::ReadyLocally => "Ready Locally",
        ProviderReadinessStatus::Partial => "Partial",
        ProviderReadinessStatus::Uninitialized
        | ProviderReadinessStatus::NotVerified
        | ProviderReadinessStatus::Failed => "Not Ready",
    };

    ProviderReadiness {
        state,
        status,
        readiness_score,
        readiness_level: readiness_level.to_string(),
        evidence: ReadinessEvidenceSummary {
            signed_report: signed_report_evidence_status,
            challenge: challenge_evidence_status,
            signed_report_freshness,
            challenge_freshness: challenge_verification
                .and_then(|verification| verification.evidence),
        },
        session,
        checks,
        warnings,
        recommendations,
    }
}

fn passed(id: &str, label: &str, max_score: u8, message: &str) -> ReadinessCheck {
    check(
        id,
        label,
        ReadinessCheckStatus::Passed,
        max_score,
        max_score,
        message,
    )
}

fn warning(id: &str, label: &str, max_score: u8, message: &str) -> ReadinessCheck {
    check(
        id,
        label,
        ReadinessCheckStatus::Warning,
        0,
        max_score,
        message,
    )
}

fn failed(id: &str, label: &str, max_score: u8, message: &str) -> ReadinessCheck {
    check(
        id,
        label,
        ReadinessCheckStatus::Failed,
        0,
        max_score,
        message,
    )
}

fn check(
    id: &str,
    label: &str,
    status: ReadinessCheckStatus,
    score: u8,
    max_score: u8,
    message: &str,
) -> ReadinessCheck {
    ReadinessCheck {
        id: id.to_string(),
        label: label.to_string(),
        status,
        score,
        max_score,
        message: message.to_string(),
    }
}

fn raw_redaction_is_safe(raw: &RawData) -> bool {
    const REQUIRED: [&str; 6] = [
        "private_key",
        "secret_key_base64",
        "private_key_path",
        "api_token",
        "api_token_hash",
        "credentials",
    ];
    raw.redacted
        && REQUIRED
            .iter()
            .all(|field| raw.redacted_fields.iter().any(|item| item == field))
        && serde_json::to_value(raw).is_ok_and(|value| sensitive_values_are_redacted(&value))
}

fn sensitive_values_are_redacted(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().all(sensitive_values_are_redacted),
        Value::Object(map) => map.iter().all(|(key, value)| {
            if is_sensitive_key(key) {
                value.is_null() || value.as_str() == Some("[redacted]")
            } else {
                sensitive_values_are_redacted(value)
            }
        }),
        _ => true,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key,
        "private_key"
            | "secret_key_base64"
            | "private_key_path"
            | "api_token"
            | "api_token_hash"
            | "credentials"
    )
}

fn deduplicate(items: &mut Vec<String>) {
    items.sort();
    items.dedup();
}
