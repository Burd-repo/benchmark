use burd_protocol::sha256_hex;
use std::env;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
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
        })
    }
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
        assert_eq!(config.admin_token_hash, sha256_hex(b"admin-secret"));
        assert!(!config.admin_token_hash.contains("admin-secret"));
    }
}
