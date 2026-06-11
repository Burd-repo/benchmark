use crate::readiness::build_provider_readiness;
use crate::report::{load_latest_signed_report, verify_signed_report_at};
use burd_hardware::{build_hardware_fingerprint_report, detect_system_report};
use burd_protocol::{
    ProviderSessionMode, ProviderSessionStatus, ProviderSessionStatusReport,
    active_provider_session, load_identity, load_latest_challenge_output, load_provider_session,
    save_provider_session, session_status_from_session,
};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderSessionExport {
    pub output: String,
    pub status: ProviderSessionStatusReport,
}

#[derive(Debug, Clone)]
pub struct ProviderSessionStartOptions {
    pub agent_version: String,
    pub host_uri: String,
}

pub fn build_provider_session_start(
    agent_version: &str,
    host_uri: &str,
) -> Result<ProviderSessionStatusReport, String> {
    let readiness = build_provider_readiness(agent_version, host_uri);

    if readiness.status != crate::readiness::ProviderReadinessStatus::ReadyLocally {
        return Err(format!(
            "provider readiness is {}; session start requires ready_locally; warnings: {}; recommendations: {}",
            readiness.status.as_str(),
            if readiness.warnings.is_empty() {
                "none".to_string()
            } else {
                readiness.warnings.join(" | ")
            },
            if readiness.recommendations.is_empty() {
                "none".to_string()
            } else {
                readiness.recommendations.join(" | ")
            }
        ));
    }

    let identity = load_identity()?;
    let system = detect_system_report(agent_version);
    let fingerprint = build_hardware_fingerprint_report(&system);
    let session_mode = if fingerprint.marketplace_policy.marketplace_eligible {
        ProviderSessionMode::MarketplaceLocal
    } else {
        ProviderSessionMode::LocalDiagnostic
    };
    let mut warnings = readiness.warnings.clone();
    if matches!(session_mode, ProviderSessionMode::LocalDiagnostic) {
        warnings.push(
            "Marketplace policy is not satisfied; starting local diagnostic session.".to_string(),
        );
    }

    let signed_report = load_latest_signed_report()?;
    let challenge_output = load_latest_challenge_output()?;
    let now = Utc::now();
    let signed_verification = verify_signed_report_at(&signed_report, now);
    if !signed_verification.signature_valid || !signed_verification.errors.is_empty() {
        return Err("latest signed report is invalid".to_string());
    }
    if signed_verification
        .evidence
        .as_ref()
        .is_some_and(|evidence| evidence.is_expired)
    {
        return Err("latest signed report is expired".to_string());
    }

    let challenge_verification = burd_protocol::verify_challenge_response(
        &challenge_output.challenge,
        &challenge_output.response,
    );
    if !challenge_verification.valid || !challenge_verification.signature_valid {
        return Err("latest challenge response is invalid".to_string());
    }
    if challenge_verification.expired {
        return Err("latest challenge response is expired".to_string());
    }

    let current_fingerprint = fingerprint.hardware_fingerprint.clone();
    let report_fingerprint = signed_report
        .report
        .hardware_fingerprint
        .clone()
        .ok_or_else(|| "latest signed report does not include hardware fingerprint".to_string())?;
    let challenge_fingerprint = challenge_output
        .response
        .hardware_fingerprint
        .clone()
        .ok_or_else(|| {
            "latest challenge response does not include hardware fingerprint".to_string()
        })?;
    if report_fingerprint != current_fingerprint {
        return Err("hardware fingerprint changed since the signed report".to_string());
    }
    if challenge_fingerprint != current_fingerprint {
        return Err("hardware fingerprint changed since the challenge response".to_string());
    }

    if let Some(existing) = load_provider_session()? {
        if existing.status == ProviderSessionStatus::Active
            && parse_timestamp(&existing.expires_at)? > now
        {
            return Err("provider session is already active".to_string());
        }
    }

    let session = active_provider_session(
        identity.provider_id.clone(),
        identity.machine_id.clone(),
        current_fingerprint.clone(),
        serde_json::to_value(&readiness)
            .map_err(|error| format!("failed to serialize readiness snapshot: {error}"))?,
        signed_report.report_hash.clone(),
        challenge_output.challenge.challenge_id.clone(),
        choose_session_expires_at(
            signed_verification
                .evidence
                .as_ref()
                .map(|evidence| evidence.expires_at.as_str()),
            challenge_verification
                .evidence
                .as_ref()
                .map(|evidence| evidence.expires_at.as_str()),
        )?,
        serde_json::to_value(&fingerprint.marketplace_policy)
            .map_err(|error| format!("failed to serialize marketplace policy: {error}"))?,
        serde_json::json!({
            "signed_report": {
                "evidence": signed_verification.evidence,
                "signature_valid": signed_verification.signature_valid,
                "expired": signed_verification.evidence.as_ref().is_some_and(|evidence| evidence.is_expired),
            },
            "challenge": {
                "evidence": challenge_verification.evidence,
                "valid": challenge_verification.valid,
                "expired": challenge_verification.expired,
            },
        }),
        session_mode,
        warnings,
    );
    save_provider_session(&session)?;

    Ok(ProviderSessionStatusReport {
        status: ProviderSessionStatus::Active,
        session: Some(session),
        online_locally: true,
        warnings: Vec::new(),
    })
}

pub fn build_provider_session_status(
    agent_version: &str,
    _host_uri: &str,
) -> Result<ProviderSessionStatusReport, String> {
    let Some(session) = load_provider_session()? else {
        return Ok(ProviderSessionStatusReport {
            status: ProviderSessionStatus::Inactive,
            session: None,
            online_locally: false,
            warnings: Vec::new(),
        });
    };

    let current_system = detect_system_report(agent_version);
    let current_fingerprint =
        build_hardware_fingerprint_report(&current_system).hardware_fingerprint;
    let mut warnings = session.warnings.clone();
    let mut status = session.status;
    let now = Utc::now();

    if session.status == ProviderSessionStatus::Stopped
        || session.status == ProviderSessionStatus::Failed
    {
    } else if parse_timestamp(&session.expires_at)? <= now {
        status = ProviderSessionStatus::Expired;
        warnings.push("provider session has expired".to_string());
    } else {
        let signed_report = load_latest_signed_report().ok();
        let challenge_output = load_latest_challenge_output().ok();
        let mut invalidated = false;

        if session.hardware_fingerprint != current_fingerprint {
            warnings.push("hardware fingerprint changed since session start".to_string());
            invalidated = true;
        }

        if let Some(report) = signed_report.as_ref() {
            let report_verification = verify_signed_report_at(report, now);
            if report.report_hash != session.report_hash
                || report.report.hardware_fingerprint.as_deref()
                    != Some(current_fingerprint.as_str())
                || !report_verification.signature_valid
                || report_verification
                    .evidence
                    .as_ref()
                    .is_none_or(|evidence| evidence.is_expired)
            {
                warnings.push("latest signed report no longer matches the session".to_string());
                invalidated = true;
            }
        } else {
            warnings.push("latest signed report is unavailable".to_string());
            invalidated = true;
        }

        if let Some(output) = challenge_output.as_ref() {
            let challenge_verification =
                burd_protocol::verify_challenge_response(&output.challenge, &output.response);
            if output.challenge.challenge_id != session.challenge_id
                || output.response.hardware_fingerprint.as_deref()
                    != Some(current_fingerprint.as_str())
                || !challenge_verification.valid
                || challenge_verification.expired
            {
                warnings
                    .push("latest challenge response no longer matches the session".to_string());
                invalidated = true;
            }
        } else {
            warnings.push("latest challenge response is unavailable".to_string());
            invalidated = true;
        }

        if invalidated {
            status = ProviderSessionStatus::Invalidated;
        } else {
            status = ProviderSessionStatus::Active;
        }
    }
    let online_locally = matches!(status, ProviderSessionStatus::Active);

    let session = session_status_from_session(session, status, online_locally);
    save_provider_session(&session)?;

    Ok(ProviderSessionStatusReport {
        status,
        session: Some(session),
        online_locally,
        warnings,
    })
}

pub fn stop_provider_session() -> Result<ProviderSessionStatusReport, String> {
    let Some(mut session) = load_provider_session()? else {
        return Ok(ProviderSessionStatusReport {
            status: ProviderSessionStatus::Inactive,
            session: None,
            online_locally: false,
            warnings: Vec::new(),
        });
    };
    session.status = ProviderSessionStatus::Stopped;
    session.online_locally = false;
    session.is_expired = false;
    save_provider_session(&session)?;
    Ok(ProviderSessionStatusReport {
        status: ProviderSessionStatus::Stopped,
        session: Some(session),
        online_locally: false,
        warnings: Vec::new(),
    })
}

pub fn export_provider_session_status(
    output: &std::path::Path,
    status: &ProviderSessionStatusReport,
) -> Result<ProviderSessionExport, String> {
    if let Some(dir) = output.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(status)
        .map_err(|error| format!("failed to serialize session status: {error}"))?;
    std::fs::write(output, json)
        .map_err(|error| format!("failed to write {}: {error}", output.display()))?;
    Ok(ProviderSessionExport {
        output: output.display().to_string(),
        status: status.clone(),
    })
}

fn choose_session_expires_at(
    signed_report_expires_at: Option<&str>,
    challenge_expires_at: Option<&str>,
) -> Result<String, String> {
    let mut expires = parse_timestamp_opt(signed_report_expires_at)?;
    if let Some(challenge_expires) = parse_timestamp_opt(challenge_expires_at)? {
        expires = Some(match expires {
            Some(current) if current < challenge_expires => current,
            Some(_) => challenge_expires,
            None => challenge_expires,
        });
    }
    expires
        .map(|value| value.to_rfc3339())
        .ok_or_else(|| "unable to determine session expiration".to_string())
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| format!("invalid timestamp '{value}': {error}"))
}

fn parse_timestamp_opt(value: Option<&str>) -> Result<Option<DateTime<Utc>>, String> {
    match value {
        Some(value) => Ok(Some(parse_timestamp(value)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[test]
    fn session_export_serializes() {
        let status = ProviderSessionStatusReport {
            status: ProviderSessionStatus::Inactive,
            session: None,
            online_locally: false,
            warnings: vec![],
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("inactive"));
    }

    #[test]
    fn session_path_uses_default_state_dir() {
        let _guard = env_lock();
        let root = std::env::temp_dir().join("burd-session-path-test");
        fs::create_dir_all(&root).unwrap();
        let env = TestEnv::new(&root);
        assert!(burd_protocol::provider_session_path().starts_with(&root));
        drop(env);
        let _ = fs::remove_dir_all(root);
    }

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    struct TestEnv {
        previous_home: Option<OsString>,
        previous_config: Option<OsString>,
    }

    impl TestEnv {
        fn new(state_dir: &PathBuf) -> Self {
            let previous_home = std::env::var_os("BURD_AGENT_HOME");
            let previous_config = std::env::var_os("BURD_AGENT_CONFIG");
            // SAFETY: session tests that mutate environment variables hold ENV_LOCK.
            unsafe {
                std::env::set_var("BURD_AGENT_HOME", state_dir);
                std::env::remove_var("BURD_AGENT_CONFIG");
            }
            Self {
                previous_home,
                previous_config,
            }
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            // SAFETY: session tests that mutate environment variables hold ENV_LOCK.
            unsafe {
                if let Some(value) = &self.previous_home {
                    std::env::set_var("BURD_AGENT_HOME", value);
                } else {
                    std::env::remove_var("BURD_AGENT_HOME");
                }
                if let Some(value) = &self.previous_config {
                    std::env::set_var("BURD_AGENT_CONFIG", value);
                } else {
                    std::env::remove_var("BURD_AGENT_CONFIG");
                }
            }
        }
    }
}
