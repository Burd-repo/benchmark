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
        let port = parse_u16(
            lookup("BURD_CONTROL_PORT").unwrap_or_else(|| "8080".to_string()),
            "BURD_CONTROL_PORT",
        )?;
        let rate_limit_per_minute = parse_u32(
            lookup("BURD_CONTROL_RATE_LIMIT_PER_MINUTE").unwrap_or_else(|| "120".to_string()),
            "BURD_CONTROL_RATE_LIMIT_PER_MINUTE",
        )?;

        Ok(Self {
            environment: lookup("BURD_CONTROL_ENV").unwrap_or_else(|| "local".to_string()),
            host: lookup("BURD_CONTROL_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            port,
            database_url,
            database_schema: lookup("BURD_CONTROL_DATABASE_SCHEMA")
                .filter(|value| !value.trim().is_empty()),
            rate_limit_per_minute,
        })
    }
}

fn parse_u16(raw: String, name: &str) -> Result<u16, ConfigError> {
    raw.parse()
        .map_err(|_| ConfigError::new(format!("{name} must be an integer between 0 and 65535")))
}

fn parse_u32(raw: String, name: &str) -> Result<u32, ConfigError> {
    raw.parse()
        .map_err(|_| ConfigError::new(format!("{name} must be a positive integer")))
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
    fn config_uses_defaults_and_database_fallback() {
        let mut values = HashMap::new();
        values.insert("DATABASE_URL", "postgres://localhost/burd");
        let config =
            ControlPlaneConfig::from_lookup(|key| values.get(key).map(|value| value.to_string()))
                .unwrap();

        assert_eq!(config.environment, "local");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert_eq!(config.database_url, "postgres://localhost/burd");
        assert_eq!(config.rate_limit_per_minute, 120);
    }
}
