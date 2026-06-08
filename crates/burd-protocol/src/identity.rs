use crate::signature::{KEY_ALGORITHM, encode_base64, generate_keypair, sha256_hex};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub provider_id: String,
    pub machine_id: String,
    pub api_url: String,
    pub preferred_provider: String,
    pub benchmark_profile: String,
    pub telemetry_enabled: bool,
    pub created_at: String,
    pub public_key: String,
    pub key_algorithm: String,
    pub private_key_path: String,
    pub email: Option<String>,
    pub website: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    #[serde(default)]
    pub api_token_hash: Option<String>,
    #[serde(default)]
    pub api_auth_enabled: bool,
    #[serde(default = "default_api_bind_host")]
    pub api_bind_host: String,
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    #[serde(default = "default_network_endpoint")]
    pub default_network_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentityPublic {
    pub provider_id: String,
    pub machine_id: String,
    pub api_url: String,
    pub preferred_provider: String,
    pub benchmark_profile: String,
    pub telemetry_enabled: bool,
    pub created_at: String,
    pub public_key: String,
    pub key_algorithm: String,
    pub email: Option<String>,
    pub website: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityInitResult {
    pub config_path: String,
    pub identity: AgentIdentityPublic,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityStatus {
    pub provider_id: String,
    pub machine_id: String,
    pub public_key: String,
    pub key_algorithm: String,
    pub created_at: String,
    pub config_path: String,
    pub private_key_path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateKeyFile {
    pub key_algorithm: String,
    pub secret_key_base64: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiTokenStatus {
    pub config_path: String,
    pub api_auth_enabled: bool,
    pub token_configured: bool,
    pub token_hash_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    pub warning: Option<String>,
}

impl AgentConfig {
    pub fn public_identity(&self) -> AgentIdentityPublic {
        AgentIdentityPublic {
            provider_id: self.provider_id.clone(),
            machine_id: self.machine_id.clone(),
            api_url: self.api_url.clone(),
            preferred_provider: self.preferred_provider.clone(),
            benchmark_profile: self.benchmark_profile.clone(),
            telemetry_enabled: self.telemetry_enabled,
            created_at: self.created_at.clone(),
            public_key: self.public_key.clone(),
            key_algorithm: self.key_algorithm.clone(),
            email: self.email.clone(),
            website: self.website.clone(),
            country: self.country.clone(),
            city: self.city.clone(),
            region: self.region.clone(),
        }
    }
}

pub fn default_state_dir() -> PathBuf {
    if let Ok(path) = std::env::var("BURD_AGENT_HOME")
        && !path.trim().is_empty()
    {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".burd")
}

pub fn default_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("BURD_AGENT_CONFIG")
        && !path.trim().is_empty()
    {
        return PathBuf::from(path);
    }

    default_state_dir().join("agent.json")
}

pub fn load_identity() -> Result<AgentConfig, String> {
    let path = default_config_path();
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("identity config not found at {}: {error}", path.display()))?;
    serde_json::from_str(&raw).map_err(|error| format!("invalid identity config JSON: {error}"))
}

pub fn load_private_key(config: &AgentConfig) -> Result<PrivateKeyFile, String> {
    let raw = fs::read_to_string(&config.private_key_path).map_err(|error| {
        format!(
            "private key not found at {}: {error}",
            config.private_key_path
        )
    })?;
    let key: PrivateKeyFile =
        serde_json::from_str(&raw).map_err(|error| format!("invalid private key JSON: {error}"))?;
    if key.key_algorithm != KEY_ALGORITHM {
        return Err(format!(
            "unsupported private key algorithm '{}'",
            key.key_algorithm
        ));
    }
    Ok(key)
}

pub fn init_identity() -> Result<IdentityInitResult, String> {
    let path = default_config_path();
    if path.exists() {
        let config = load_identity()?;
        return Ok(IdentityInitResult {
            config_path: path.display().to_string(),
            identity: config.public_identity(),
            created: false,
        });
    }

    let dir = path
        .parent()
        .ok_or_else(|| "cannot resolve Burd agent config directory".to_string())?;
    fs::create_dir_all(dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;

    let created_at = Utc::now().to_rfc3339();
    let private_key_path = dir.join("agent.key");
    let keypair = generate_keypair()?;
    let private_key = PrivateKeyFile {
        key_algorithm: KEY_ALGORITHM.to_string(),
        secret_key_base64: keypair.secret_key_base64,
        created_at: created_at.clone(),
    };
    write_private_key(&private_key_path, &private_key)?;

    let config = AgentConfig {
        provider_id: format!("burd-provider-{}", Uuid::new_v4()),
        machine_id: format!("burd-machine-{}", Uuid::new_v4()),
        api_url: "https://api.burd.cloud".to_string(),
        preferred_provider: "ollama".to_string(),
        benchmark_profile: "auto".to_string(),
        telemetry_enabled: false,
        created_at,
        public_key: keypair.public_key_base64,
        key_algorithm: KEY_ALGORITHM.to_string(),
        private_key_path: private_key_path.display().to_string(),
        email: None,
        website: None,
        country: None,
        city: None,
        region: None,
        api_token_hash: None,
        api_auth_enabled: false,
        api_bind_host: default_api_bind_host(),
        api_port: default_api_port(),
        default_network_endpoint: default_network_endpoint(),
    };

    write_config(&path, &config)?;

    Ok(IdentityInitResult {
        config_path: path.display().to_string(),
        identity: config.public_identity(),
        created: true,
    })
}

pub fn show_identity() -> Result<IdentityStatus, String> {
    let config = load_identity()?;
    Ok(identity_status(&config, &default_config_path()))
}

pub fn rotate_identity_key(confirm: bool) -> Result<IdentityStatus, String> {
    if !confirm {
        return Err("key rotation requires --confirm".to_string());
    }

    let path = default_config_path();
    let mut config = load_identity()?;
    let keypair = generate_keypair()?;
    let private_key = PrivateKeyFile {
        key_algorithm: KEY_ALGORITHM.to_string(),
        secret_key_base64: keypair.secret_key_base64,
        created_at: Utc::now().to_rfc3339(),
    };
    write_private_key(
        PathBuf::from(&config.private_key_path).as_path(),
        &private_key,
    )?;
    config.public_key = keypair.public_key_base64;
    config.key_algorithm = KEY_ALGORITHM.to_string();
    write_config(&path, &config)?;
    Ok(identity_status(&config, &path))
}

pub fn create_api_token() -> Result<ApiTokenStatus, String> {
    update_api_token(true)
}

pub fn rotate_api_token() -> Result<ApiTokenStatus, String> {
    update_api_token(true)
}

pub fn show_api_token_status() -> Result<ApiTokenStatus, String> {
    let config = load_identity()?;
    Ok(api_token_status(&config, None))
}

pub fn verify_api_token(token: &str) -> Result<bool, String> {
    let config = load_identity()?;
    if !config.api_auth_enabled {
        return Ok(true);
    }
    let Some(expected_hash) = config.api_token_hash else {
        return Ok(false);
    };
    Ok(sha256_hex(token.as_bytes()) == expected_hash)
}

pub fn redacted_config_value() -> Result<Value, String> {
    let config = load_identity()?;
    Ok(serde_json::json!({
        "provider_id": config.provider_id,
        "machine_id": config.machine_id,
        "api_url": config.api_url,
        "preferred_provider": config.preferred_provider,
        "benchmark_profile": config.benchmark_profile,
        "telemetry_enabled": config.telemetry_enabled,
        "created_at": config.created_at,
        "public_key": config.public_key,
        "key_algorithm": config.key_algorithm,
        "private_key_path": "[redacted]",
        "email": config.email,
        "website": config.website,
        "country": config.country,
        "city": config.city,
        "region": config.region,
        "api_token_hash": config.api_token_hash.as_ref().map(|_| "[redacted]"),
        "api_auth_enabled": config.api_auth_enabled,
        "api_bind_host": config.api_bind_host,
        "api_port": config.api_port,
        "default_network_endpoint": config.default_network_endpoint,
    }))
}

fn update_api_token(include_token: bool) -> Result<ApiTokenStatus, String> {
    if !default_config_path().exists() {
        let _ = init_identity()?;
    }
    let path = default_config_path();
    let mut config = load_identity()?;
    let token = generate_api_token()?;
    config.api_token_hash = Some(sha256_hex(token.as_bytes()));
    config.api_auth_enabled = true;
    write_config(&path, &config)?;
    Ok(api_token_status(
        &config,
        if include_token { Some(token) } else { None },
    ))
}

fn api_token_status(config: &AgentConfig, token: Option<String>) -> ApiTokenStatus {
    ApiTokenStatus {
        config_path: default_config_path().display().to_string(),
        api_auth_enabled: config.api_auth_enabled,
        token_configured: config.api_token_hash.is_some(),
        token_hash_preview: config
            .api_token_hash
            .as_ref()
            .map(|hash| format!("{}...", &hash[..hash.len().min(12)])),
        token,
        warning: if config.api_auth_enabled {
            None
        } else {
            Some("local API authentication is disabled".to_string())
        },
    }
}

fn generate_api_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("failed to generate API token: {error}"))?;
    Ok(encode_base64(&bytes))
}

fn identity_status(config: &AgentConfig, config_path: &std::path::Path) -> IdentityStatus {
    let private_key_status = if std::path::Path::new(&config.private_key_path).exists() {
        "ready"
    } else {
        "missing_private_key"
    };
    IdentityStatus {
        provider_id: config.provider_id.clone(),
        machine_id: config.machine_id.clone(),
        public_key: config.public_key.clone(),
        key_algorithm: config.key_algorithm.clone(),
        created_at: config.created_at.clone(),
        config_path: config_path.display().to_string(),
        private_key_path: config.private_key_path.clone(),
        status: private_key_status.to_string(),
    }
}

fn write_config(path: &std::path::Path, config: &AgentConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config)
        .map_err(|error| format!("failed to serialize identity config: {error}"))?;
    fs::write(path, json).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn write_private_key(path: &std::path::Path, key: &PrivateKeyFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(key)
        .map_err(|error| format!("failed to serialize private key: {error}"))?;
    fs::write(path, json)
        .map_err(|error| format!("failed to write private key at {}: {error}", path.display()))
}

fn default_api_bind_host() -> String {
    "127.0.0.1".to_string()
}

fn default_api_port() -> u16 {
    8787
}

fn default_network_endpoint() -> String {
    "https://www.cloudflare.com/cdn-cgi/trace".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_identity_hides_private_key() {
        let config = AgentConfig {
            provider_id: "provider".to_string(),
            machine_id: "machine".to_string(),
            api_url: "https://api.example".to_string(),
            preferred_provider: "ollama".to_string(),
            benchmark_profile: "profile_12gb".to_string(),
            telemetry_enabled: false,
            created_at: "2026-06-08T00:00:00Z".to_string(),
            public_key: "pub".to_string(),
            key_algorithm: KEY_ALGORITHM.to_string(),
            private_key_path: "/secret".to_string(),
            email: None,
            website: None,
            country: None,
            city: None,
            region: None,
            api_token_hash: None,
            api_auth_enabled: false,
            api_bind_host: default_api_bind_host(),
            api_port: default_api_port(),
            default_network_endpoint: default_network_endpoint(),
        };

        let json = serde_json::to_string(&config.public_identity()).unwrap();
        assert!(!json.contains("private_key_path"));
        assert!(!json.contains("/secret"));
    }

    #[test]
    fn config_json_roundtrip() {
        let config = AgentConfig {
            provider_id: "provider".to_string(),
            machine_id: "machine".to_string(),
            api_url: "https://api.example".to_string(),
            preferred_provider: "ollama".to_string(),
            benchmark_profile: "auto".to_string(),
            telemetry_enabled: true,
            created_at: "2026-06-08T00:00:00Z".to_string(),
            public_key: "pub".to_string(),
            key_algorithm: KEY_ALGORITHM.to_string(),
            private_key_path: "/secret".to_string(),
            email: Some("ops@example.com".to_string()),
            website: None,
            country: Some("BR".to_string()),
            city: Some("SAO".to_string()),
            region: Some("br-southeast".to_string()),
            api_token_hash: Some("hash".to_string()),
            api_auth_enabled: true,
            api_bind_host: "127.0.0.1".to_string(),
            api_port: 8787,
            default_network_endpoint: default_network_endpoint(),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.provider_id, "provider");
        assert_eq!(parsed.machine_id, "machine");
    }

    #[test]
    fn generated_api_token_is_not_empty() {
        let token = generate_api_token().unwrap();
        assert!(token.len() > 32);
        assert_ne!(sha256_hex(token.as_bytes()), token);
    }
}
