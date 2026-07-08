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
    pub rate_limit_per_minute: u32,
    pub admin_token_hash: String,
    pub enrollment_token_ttl_seconds: u32,
    pub enrollment_proof_ttl_seconds: u32,
    pub device_credential_ttl_seconds: u32,
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

        Ok(Self {
            environment: lookup("BURD_CONTROL_ENV").unwrap_or_else(|| "local".to_string()),
            host: lookup("BURD_CONTROL_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            port,
            database_url,
            database_schema: lookup("BURD_CONTROL_DATABASE_SCHEMA")
                .filter(|value| !value.trim().is_empty()),
            rate_limit_per_minute,
            admin_token_hash: sha256_hex(admin_token.as_bytes()),
            enrollment_token_ttl_seconds,
            enrollment_proof_ttl_seconds,
            device_credential_ttl_seconds,
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
        assert_eq!(config.rate_limit_per_minute, 120);
        assert_eq!(config.enrollment_token_ttl_seconds, 600);
        assert_eq!(config.enrollment_proof_ttl_seconds, 300);
        assert_eq!(config.device_credential_ttl_seconds, 900);
        assert_eq!(config.admin_token_hash, sha256_hex(b"admin-secret"));
        assert!(!config.admin_token_hash.contains("admin-secret"));
    }
}
