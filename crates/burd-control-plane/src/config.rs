use burd_protocol::{PROOF_CAPABILITY_REQUIRED_PROOFS, sha256_hex};
use std::collections::HashSet;
use std::env;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct ControlPlaneConfig {
    pub environment: String,
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub database_schema: Option<String>,
    pub object_storage_dir: String,
    pub rate_limit_per_minute: u32,
    pub admin_token_hash: String,
    pub enrollment_token_ttl_seconds: u32,
    pub enrollment_proof_ttl_seconds: u32,
    pub device_credential_ttl_seconds: u32,
    pub remote_session_ttl_seconds: u32,
    pub heartbeat_interval_seconds: u32,
    pub missed_heartbeat_limit: u32,
    pub telemetry_max_samples_per_batch: u32,
    pub telemetry_min_batch_interval_seconds: u32,
    pub telemetry_clock_skew_seconds: u32,
    pub telemetry_retention_days: u32,
    pub proof_challenge_ttl_seconds: u32,
    pub proof_challenge_clock_skew_seconds: u32,
    pub verification_period_seconds: u32,
    pub verification_retry_budget: u32,
    pub verification_sweep_limit: u32,
    pub verification_suspect_failures: u32,
    pub verification_proof_profile: Option<VerificationProofProfileConfig>,
    pub observability_deployment_id: String,
    pub observability_recent_events_limit: u32,
    pub slo_availability_target_bps: u32,
    pub slo_p95_latency_ms: u32,
    pub security_min_agent_version: Option<String>,
    pub security_require_signed_agent_release: bool,
    pub security_require_hardware_backed_key: bool,
    pub security_require_remote_attestation: bool,
    pub security_require_sbom_hash: bool,
    pub security_accepted_release_channels: Vec<String>,
    pub security_accepted_attestation_modes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationProofProfileConfig {
    pub profile_version: String,
    pub model_artifact_hash: String,
    pub required_proofs: Vec<String>,
    pub min_tokens_per_second: f64,
    pub max_ttft_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

impl ControlPlaneConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    pub fn from_lookup<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let database_url = lookup("BURD_CONTROL_DATABASE_URL")
            .or_else(|| lookup("DATABASE_URL"))
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                ConfigError::new(
                    "BURD_CONTROL_DATABASE_URL or DATABASE_URL must point to PostgreSQL",
                )
            })?;
        let object_storage_dir = lookup("BURD_CONTROL_OBJECT_STORAGE_DIR")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "./.burd-control-objects".to_string());
        let admin_token = lookup("BURD_CONTROL_ADMIN_TOKEN")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ConfigError::new("BURD_CONTROL_ADMIN_TOKEN is required"))?;
        let port = parse_u16(
            lookup("BURD_CONTROL_PORT").unwrap_or_else(|| "8080".to_string()),
            "BURD_CONTROL_PORT",
        )?;
        let rate_limit_per_minute = parse_u32(
            lookup("BURD_CONTROL_RATE_LIMIT_PER_MINUTE").unwrap_or_else(|| "120".to_string()),
            "BURD_CONTROL_RATE_LIMIT_PER_MINUTE",
        )?;
        let enrollment_token_ttl_seconds = parse_u32(
            lookup("BURD_CONTROL_ENROLLMENT_TOKEN_TTL_SECONDS")
                .unwrap_or_else(|| "600".to_string()),
            "BURD_CONTROL_ENROLLMENT_TOKEN_TTL_SECONDS",
        )?;
        let enrollment_proof_ttl_seconds = parse_u32(
            lookup("BURD_CONTROL_ENROLLMENT_PROOF_TTL_SECONDS")
                .unwrap_or_else(|| "300".to_string()),
            "BURD_CONTROL_ENROLLMENT_PROOF_TTL_SECONDS",
        )?;
        let device_credential_ttl_seconds = parse_u32(
            lookup("BURD_CONTROL_DEVICE_CREDENTIAL_TTL_SECONDS")
                .unwrap_or_else(|| "900".to_string()),
            "BURD_CONTROL_DEVICE_CREDENTIAL_TTL_SECONDS",
        )?;
        let remote_session_ttl_seconds = parse_u32(
            lookup("BURD_CONTROL_SESSION_TTL_SECONDS").unwrap_or_else(|| "900".to_string()),
            "BURD_CONTROL_SESSION_TTL_SECONDS",
        )?;
        let heartbeat_interval_seconds = parse_u32(
            lookup("BURD_CONTROL_HEARTBEAT_INTERVAL_SECONDS").unwrap_or_else(|| "15".to_string()),
            "BURD_CONTROL_HEARTBEAT_INTERVAL_SECONDS",
        )?;
        let missed_heartbeat_limit = parse_u32(
            lookup("BURD_CONTROL_MISSED_HEARTBEAT_LIMIT").unwrap_or_else(|| "3".to_string()),
            "BURD_CONTROL_MISSED_HEARTBEAT_LIMIT",
        )?;
        let telemetry_max_samples_per_batch = parse_u32(
            lookup("BURD_CONTROL_TELEMETRY_MAX_SAMPLES_PER_BATCH")
                .unwrap_or_else(|| "64".to_string()),
            "BURD_CONTROL_TELEMETRY_MAX_SAMPLES_PER_BATCH",
        )?;
        let telemetry_min_batch_interval_seconds = parse_u32(
            lookup("BURD_CONTROL_TELEMETRY_MIN_BATCH_INTERVAL_SECONDS")
                .unwrap_or_else(|| "5".to_string()),
            "BURD_CONTROL_TELEMETRY_MIN_BATCH_INTERVAL_SECONDS",
        )?;
        let telemetry_clock_skew_seconds = parse_u32(
            lookup("BURD_CONTROL_TELEMETRY_CLOCK_SKEW_SECONDS")
                .unwrap_or_else(|| "300".to_string()),
            "BURD_CONTROL_TELEMETRY_CLOCK_SKEW_SECONDS",
        )?;
        let telemetry_retention_days = parse_u32(
            lookup("BURD_CONTROL_TELEMETRY_RETENTION_DAYS").unwrap_or_else(|| "7".to_string()),
            "BURD_CONTROL_TELEMETRY_RETENTION_DAYS",
        )?;
        let proof_challenge_ttl_seconds = parse_u32(
            lookup("BURD_CONTROL_PROOF_CHALLENGE_TTL_SECONDS").unwrap_or_else(|| "600".to_string()),
            "BURD_CONTROL_PROOF_CHALLENGE_TTL_SECONDS",
        )?;
        let proof_challenge_clock_skew_seconds = parse_u32(
            lookup("BURD_CONTROL_PROOF_CHALLENGE_CLOCK_SKEW_SECONDS")
                .unwrap_or_else(|| "300".to_string()),
            "BURD_CONTROL_PROOF_CHALLENGE_CLOCK_SKEW_SECONDS",
        )?;
        let verification_period_seconds = parse_u32(
            lookup("BURD_CONTROL_VERIFICATION_PERIOD_SECONDS")
                .unwrap_or_else(|| "3600".to_string()),
            "BURD_CONTROL_VERIFICATION_PERIOD_SECONDS",
        )?;
        let verification_retry_budget = parse_u32(
            lookup("BURD_CONTROL_VERIFICATION_RETRY_BUDGET").unwrap_or_else(|| "2".to_string()),
            "BURD_CONTROL_VERIFICATION_RETRY_BUDGET",
        )?;
        let verification_sweep_limit = parse_u32(
            lookup("BURD_CONTROL_VERIFICATION_SWEEP_LIMIT").unwrap_or_else(|| "25".to_string()),
            "BURD_CONTROL_VERIFICATION_SWEEP_LIMIT",
        )?;
        let verification_suspect_failures = parse_u32(
            lookup("BURD_CONTROL_VERIFICATION_SUSPECT_FAILURES").unwrap_or_else(|| "3".to_string()),
            "BURD_CONTROL_VERIFICATION_SUSPECT_FAILURES",
        )?;
        let verification_profile_version = lookup("BURD_CONTROL_VERIFICATION_PROFILE_VERSION")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "poc-cuda-llm-v1".to_string());
        let verification_model_artifact_hash =
            lookup("BURD_CONTROL_VERIFICATION_MODEL_ARTIFACT_HASH")
                .filter(|value| !value.trim().is_empty());
        let verification_required_proofs = parse_csv(
            lookup("BURD_CONTROL_VERIFICATION_REQUIRED_PROOFS")
                .unwrap_or_else(|| PROOF_CAPABILITY_REQUIRED_PROOFS.join(",")),
            "BURD_CONTROL_VERIFICATION_REQUIRED_PROOFS",
        )?;
        let verification_min_tokens_per_second = parse_nonnegative_f64(
            lookup("BURD_CONTROL_VERIFICATION_MIN_TOKENS_PER_SECOND")
                .unwrap_or_else(|| "0".to_string()),
            "BURD_CONTROL_VERIFICATION_MIN_TOKENS_PER_SECOND",
        )?;
        let verification_max_ttft_ms = parse_nonnegative_u64(
            lookup("BURD_CONTROL_VERIFICATION_MAX_TTFT_MS").unwrap_or_else(|| "0".to_string()),
            "BURD_CONTROL_VERIFICATION_MAX_TTFT_MS",
        )?;
        let verification_proof_profile = build_verification_proof_profile(
            verification_profile_version,
            verification_model_artifact_hash,
            verification_required_proofs,
            verification_min_tokens_per_second,
            verification_max_ttft_ms,
        )?;

        let observability_recent_events_limit = parse_u32(
            lookup("BURD_CONTROL_OBSERVABILITY_RECENT_EVENTS_LIMIT")
                .unwrap_or_else(|| "100".to_string()),
            "BURD_CONTROL_OBSERVABILITY_RECENT_EVENTS_LIMIT",
        )?;
        let slo_availability_target_bps = parse_bps(
            lookup("BURD_CONTROL_SLO_AVAILABILITY_TARGET_BPS")
                .unwrap_or_else(|| "9990".to_string()),
            "BURD_CONTROL_SLO_AVAILABILITY_TARGET_BPS",
        )?;
        let slo_p95_latency_ms = parse_u32(
            lookup("BURD_CONTROL_SLO_P95_LATENCY_MS").unwrap_or_else(|| "500".to_string()),
            "BURD_CONTROL_SLO_P95_LATENCY_MS",
        )?;
        let security_require_signed_agent_release = parse_bool(
            lookup("BURD_CONTROL_SECURITY_REQUIRE_SIGNED_AGENT_RELEASE")
                .unwrap_or_else(|| "false".to_string()),
            "BURD_CONTROL_SECURITY_REQUIRE_SIGNED_AGENT_RELEASE",
        )?;
        let security_require_hardware_backed_key = parse_bool(
            lookup("BURD_CONTROL_SECURITY_REQUIRE_HARDWARE_BACKED_KEY")
                .unwrap_or_else(|| "false".to_string()),
            "BURD_CONTROL_SECURITY_REQUIRE_HARDWARE_BACKED_KEY",
        )?;
        let security_require_remote_attestation = parse_bool(
            lookup("BURD_CONTROL_SECURITY_REQUIRE_REMOTE_ATTESTATION")
                .unwrap_or_else(|| "false".to_string()),
            "BURD_CONTROL_SECURITY_REQUIRE_REMOTE_ATTESTATION",
        )?;
        let security_require_sbom_hash = parse_bool(
            lookup("BURD_CONTROL_SECURITY_REQUIRE_SBOM_HASH")
                .unwrap_or_else(|| "false".to_string()),
            "BURD_CONTROL_SECURITY_REQUIRE_SBOM_HASH",
        )?;
        let security_accepted_release_channels = parse_csv(
            lookup("BURD_CONTROL_SECURITY_ACCEPTED_RELEASE_CHANNELS")
                .unwrap_or_else(|| "dev,stable".to_string()),
            "BURD_CONTROL_SECURITY_ACCEPTED_RELEASE_CHANNELS",
        )?;
        let security_accepted_attestation_modes = parse_csv(
            lookup("BURD_CONTROL_SECURITY_ACCEPTED_ATTESTATION_MODES")
                .unwrap_or_else(|| "none,tpm,os_keychain,hsm,sev_snp,sgx".to_string()),
            "BURD_CONTROL_SECURITY_ACCEPTED_ATTESTATION_MODES",
        )?;
        Ok(Self {
            environment: lookup("BURD_CONTROL_ENV").unwrap_or_else(|| "local".to_string()),
            host: lookup("BURD_CONTROL_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            port,
            database_url,
            database_schema: lookup("BURD_CONTROL_DATABASE_SCHEMA")
                .filter(|value| !value.trim().is_empty()),
            object_storage_dir,
            rate_limit_per_minute,
            admin_token_hash: sha256_hex(admin_token.as_bytes()),
            enrollment_token_ttl_seconds,
            enrollment_proof_ttl_seconds,
            device_credential_ttl_seconds,
            remote_session_ttl_seconds,
            heartbeat_interval_seconds,
            missed_heartbeat_limit,
            telemetry_max_samples_per_batch,
            telemetry_min_batch_interval_seconds,
            telemetry_clock_skew_seconds,
            telemetry_retention_days,
            proof_challenge_ttl_seconds,
            proof_challenge_clock_skew_seconds,
            verification_period_seconds,
            verification_retry_budget,
            verification_sweep_limit,
            verification_suspect_failures,
            verification_proof_profile,
            observability_deployment_id: lookup("BURD_CONTROL_DEPLOYMENT_ID")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "local".to_string()),
            observability_recent_events_limit,
            slo_availability_target_bps,
            slo_p95_latency_ms,
            security_min_agent_version: lookup("BURD_CONTROL_SECURITY_MIN_AGENT_VERSION")
                .filter(|value| !value.trim().is_empty()),
            security_require_signed_agent_release,
            security_require_hardware_backed_key,
            security_require_remote_attestation,
            security_require_sbom_hash,
            security_accepted_release_channels,
            security_accepted_attestation_modes,
        })
    }
}

fn build_verification_proof_profile(
    profile_version: String,
    model_artifact_hash: Option<String>,
    required_proofs: Vec<String>,
    min_tokens_per_second: f64,
    max_ttft_ms: u64,
) -> Result<Option<VerificationProofProfileConfig>, ConfigError> {
    let profile_version = profile_version.trim().to_string();
    if !is_bounded_ascii(&profile_version, 96) {
        return Err(ConfigError::new(
            "BURD_CONTROL_VERIFICATION_PROFILE_VERSION must be short printable ASCII",
        ));
    }

    let unique_proofs = required_proofs
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let has_exact_proof_contract = unique_proofs.len() == required_proofs.len()
        && unique_proofs.len() == PROOF_CAPABILITY_REQUIRED_PROOFS.len()
        && PROOF_CAPABILITY_REQUIRED_PROOFS
            .iter()
            .all(|proof| unique_proofs.contains(proof));
    if !has_exact_proof_contract {
        return Err(ConfigError::new(format!(
            "BURD_CONTROL_VERIFICATION_REQUIRED_PROOFS must contain each supported proof exactly once: {}",
            PROOF_CAPABILITY_REQUIRED_PROOFS.join(",")
        )));
    }

    let Some(model_artifact_hash) = model_artifact_hash else {
        if min_tokens_per_second > 0.0 || max_ttft_ms > 0 {
            return Err(ConfigError::new(
                "BURD_CONTROL_VERIFICATION_MODEL_ARTIFACT_HASH is required when verification thresholds are configured",
            ));
        }
        return Ok(None);
    };
    let model_artifact_hash = model_artifact_hash.trim().to_ascii_lowercase();
    if !is_sha256_digest(&model_artifact_hash) {
        return Err(ConfigError::new(
            "BURD_CONTROL_VERIFICATION_MODEL_ARTIFACT_HASH must be an exact sha256 digest",
        ));
    }
    if min_tokens_per_second <= 0.0 {
        return Err(ConfigError::new(
            "BURD_CONTROL_VERIFICATION_MIN_TOKENS_PER_SECOND must be greater than zero when a proof profile is configured",
        ));
    }
    if max_ttft_ms == 0 || max_ttft_ms > i64::MAX as u64 {
        return Err(ConfigError::new(
            "BURD_CONTROL_VERIFICATION_MAX_TTFT_MS must be between 1 and i64::MAX when a proof profile is configured",
        ));
    }

    Ok(Some(VerificationProofProfileConfig {
        profile_version,
        model_artifact_hash,
        required_proofs,
        min_tokens_per_second,
        max_ttft_ms,
    }))
}

fn parse_nonnegative_f64(raw: String, name: &str) -> Result<f64, ConfigError> {
    let value = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| ConfigError::new(format!("{name} must be a nonnegative number")))?;
    if !value.is_finite() || value < 0.0 {
        return Err(ConfigError::new(format!(
            "{name} must be a nonnegative finite number"
        )));
    }
    Ok(value)
}

fn parse_nonnegative_u64(raw: String, name: &str) -> Result<u64, ConfigError> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| ConfigError::new(format!("{name} must be a nonnegative integer")))
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn is_bounded_ascii(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .chars()
            .all(|character| character.is_ascii() && !character.is_ascii_control())
}

fn parse_u16(raw: String, name: &str) -> Result<u16, ConfigError> {
    raw.parse()
        .map_err(|_| ConfigError::new(format!("{name} must be an integer between 0 and 65535")))
}

fn parse_u32(raw: String, name: &str) -> Result<u32, ConfigError> {
    let value = raw
        .parse()
        .map_err(|_| ConfigError::new(format!("{name} must be a positive integer")))?;
    if value == 0 {
        return Err(ConfigError::new(format!(
            "{name} must be a positive integer"
        )));
    }
    Ok(value)
}

fn parse_bps(raw: String, name: &str) -> Result<u32, ConfigError> {
    let value = parse_u32(raw, name)?;
    if value > 10_000 {
        return Err(ConfigError::new(format!("{name} must be at most 10000")));
    }
    Ok(value)
}

fn parse_bool(raw: String, name: &str) -> Result<bool, ConfigError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::new(format!("{name} must be a boolean value"))),
    }
}

fn parse_csv(raw: String, name: &str) -> Result<Vec<String>, ConfigError> {
    let values = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Err(ConfigError::new(format!("{name} must not be empty")));
    }
    Ok(values)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn config_requires_database_url() {
        let error = ControlPlaneConfig::from_lookup(|_| None).unwrap_err();
        assert!(error.to_string().contains("DATABASE_URL"));
    }

    #[test]
    fn config_requires_admin_token_after_database_url() {
        let error = ControlPlaneConfig::from_lookup(|key| {
            (key == "DATABASE_URL").then(|| "postgres://localhost/burd".to_string())
        })
        .unwrap_err();
        assert!(error.to_string().contains("ADMIN_TOKEN"));
    }

    #[test]
    fn config_uses_defaults_and_database_fallback() {
        let mut values = HashMap::new();
        values.insert("DATABASE_URL", "postgres://localhost/burd");
        values.insert("BURD_CONTROL_ADMIN_TOKEN", "admin-secret");
        let config =
            ControlPlaneConfig::from_lookup(|key| values.get(key).map(|value| value.to_string()))
                .unwrap();

        assert_eq!(config.environment, "local");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert_eq!(config.database_url, "postgres://localhost/burd");
        assert_eq!(config.object_storage_dir, "./.burd-control-objects");
        assert_eq!(config.rate_limit_per_minute, 120);
        assert_eq!(config.enrollment_token_ttl_seconds, 600);
        assert_eq!(config.enrollment_proof_ttl_seconds, 300);
        assert_eq!(config.device_credential_ttl_seconds, 900);
        assert_eq!(config.remote_session_ttl_seconds, 900);
        assert_eq!(config.heartbeat_interval_seconds, 15);
        assert_eq!(config.missed_heartbeat_limit, 3);
        assert_eq!(config.telemetry_max_samples_per_batch, 64);
        assert_eq!(config.telemetry_min_batch_interval_seconds, 5);
        assert_eq!(config.telemetry_clock_skew_seconds, 300);
        assert_eq!(config.telemetry_retention_days, 7);
        assert_eq!(config.proof_challenge_ttl_seconds, 600);
        assert_eq!(config.proof_challenge_clock_skew_seconds, 300);
        assert_eq!(config.verification_period_seconds, 3600);
        assert_eq!(config.verification_retry_budget, 2);
        assert_eq!(config.verification_sweep_limit, 25);
        assert_eq!(config.verification_suspect_failures, 3);
        assert!(config.verification_proof_profile.is_none());
        assert_eq!(config.observability_deployment_id, "local");
        assert_eq!(config.observability_recent_events_limit, 100);
        assert_eq!(config.slo_availability_target_bps, 9990);
        assert_eq!(config.slo_p95_latency_ms, 500);
        assert_eq!(config.security_min_agent_version, None);
        assert!(!config.security_require_signed_agent_release);
        assert!(!config.security_require_hardware_backed_key);
        assert!(!config.security_require_remote_attestation);
        assert!(!config.security_require_sbom_hash);
        assert_eq!(
            config.security_accepted_release_channels,
            vec!["dev", "stable"]
        );
        assert_eq!(
            config.security_accepted_attestation_modes,
            vec!["none", "tpm", "os_keychain", "hsm", "sev_snp", "sgx"]
        );
        assert_eq!(config.admin_token_hash, sha256_hex(b"admin-secret"));
        assert!(!config.admin_token_hash.contains("admin-secret"));
    }

    #[test]
    fn config_rejects_invalid_slo_bps() {
        let mut values = HashMap::new();
        values.insert("DATABASE_URL", "postgres://localhost/burd");
        values.insert("BURD_CONTROL_ADMIN_TOKEN", "admin-secret");
        values.insert("BURD_CONTROL_SLO_AVAILABILITY_TARGET_BPS", "10001");
        let error =
            ControlPlaneConfig::from_lookup(|key| values.get(key).map(|value| value.to_string()))
                .unwrap_err();
        assert!(error.to_string().contains("SLO_AVAILABILITY"));
    }

    #[test]
    fn config_rejects_invalid_security_boolean() {
        let mut values = HashMap::new();
        values.insert("DATABASE_URL", "postgres://localhost/burd");
        values.insert("BURD_CONTROL_ADMIN_TOKEN", "admin-secret");
        values.insert("BURD_CONTROL_SECURITY_REQUIRE_REMOTE_ATTESTATION", "maybe");
        let error =
            ControlPlaneConfig::from_lookup(|key| values.get(key).map(|value| value.to_string()))
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("SECURITY_REQUIRE_REMOTE_ATTESTATION")
        );
    }
    fn base_values() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("DATABASE_URL", "postgres://localhost/burd"),
            ("BURD_CONTROL_ADMIN_TOKEN", "admin-secret"),
        ])
    }

    #[test]
    fn config_builds_complete_versioned_verification_profile() {
        let mut values = base_values();
        values.insert(
            "BURD_CONTROL_VERIFICATION_PROFILE_VERSION",
            "poc-cuda-llm-v2",
        );
        values.insert(
            "BURD_CONTROL_VERIFICATION_MODEL_ARTIFACT_HASH",
            "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        );
        values.insert("BURD_CONTROL_VERIFICATION_MIN_TOKENS_PER_SECOND", "12.5");
        values.insert("BURD_CONTROL_VERIFICATION_MAX_TTFT_MS", "1500");
        let config = ControlPlaneConfig::from_lookup(|key| {
            values.get(key).map(|value| (*value).to_string())
        })
        .unwrap();

        let profile = config.verification_proof_profile.unwrap();
        assert_eq!(profile.profile_version, "poc-cuda-llm-v2");
        assert_eq!(
            profile.model_artifact_hash,
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(profile.required_proofs.len(), 7);
        assert_eq!(profile.min_tokens_per_second, 12.5);
        assert_eq!(profile.max_ttft_ms, 1500);
    }

    #[test]
    fn config_rejects_partial_verification_profile() {
        let mut values = base_values();
        values.insert("BURD_CONTROL_VERIFICATION_MIN_TOKENS_PER_SECOND", "12.5");
        values.insert("BURD_CONTROL_VERIFICATION_MAX_TTFT_MS", "1500");
        let error = ControlPlaneConfig::from_lookup(|key| {
            values.get(key).map(|value| (*value).to_string())
        })
        .unwrap_err();
        assert!(error.to_string().contains("MODEL_ARTIFACT_HASH"));

        values.insert(
            "BURD_CONTROL_VERIFICATION_MODEL_ARTIFACT_HASH",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        values.insert("BURD_CONTROL_VERIFICATION_MIN_TOKENS_PER_SECOND", "0");
        let error = ControlPlaneConfig::from_lookup(|key| {
            values.get(key).map(|value| (*value).to_string())
        })
        .unwrap_err();
        assert!(error.to_string().contains("MIN_TOKENS_PER_SECOND"));

        values.insert("BURD_CONTROL_VERIFICATION_MIN_TOKENS_PER_SECOND", "12.5");
        values.insert(
            "BURD_CONTROL_VERIFICATION_MAX_TTFT_MS",
            "18446744073709551615",
        );
        let error = ControlPlaneConfig::from_lookup(|key| {
            values.get(key).map(|value| (*value).to_string())
        })
        .unwrap_err();
        assert!(error.to_string().contains("MAX_TTFT_MS"));
    }

    #[test]
    fn config_rejects_placeholder_digest_and_incomplete_proof_contract() {
        let mut values = base_values();
        values.insert(
            "BURD_CONTROL_VERIFICATION_MODEL_ARTIFACT_HASH",
            "sha256:burd-poc-v1",
        );
        values.insert("BURD_CONTROL_VERIFICATION_MIN_TOKENS_PER_SECOND", "12.5");
        values.insert("BURD_CONTROL_VERIFICATION_MAX_TTFT_MS", "1500");
        let error = ControlPlaneConfig::from_lookup(|key| {
            values.get(key).map(|value| (*value).to_string())
        })
        .unwrap_err();
        assert!(error.to_string().contains("exact sha256 digest"));

        values.insert(
            "BURD_CONTROL_VERIFICATION_MODEL_ARTIFACT_HASH",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        values.insert(
            "BURD_CONTROL_VERIFICATION_REQUIRED_PROOFS",
            "cuda_runtime,llm_short_inference",
        );
        let error = ControlPlaneConfig::from_lookup(|key| {
            values.get(key).map(|value| (*value).to_string())
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("each supported proof exactly once")
        );
    }
}
