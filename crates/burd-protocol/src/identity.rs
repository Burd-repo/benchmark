use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub provider_id: Option<String>,
    pub machine_id: String,
    pub api_url: String,
    pub preferred_provider: String,
    pub benchmark_profile: String,
    pub telemetry_enabled: bool,
    pub created_at: String,
    pub public_key: String,
    pub private_key_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentityPublic {
    pub provider_id: Option<String>,
    pub machine_id: String,
    pub api_url: String,
    pub preferred_provider: String,
    pub benchmark_profile: String,
    pub telemetry_enabled: bool,
    pub created_at: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityInitResult {
    pub config_path: String,
    pub identity: AgentIdentityPublic,
    pub created: bool,
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
        }
    }
}

pub fn default_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("BURD_AGENT_CONFIG")
        && !path.trim().is_empty()
    {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".burd")
        .join("agent.json")
}

pub fn load_identity() -> Result<AgentConfig, String> {
    let path = default_config_path();
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("identity config not found at {}: {error}", path.display()))?;
    serde_json::from_str(&raw).map_err(|error| format!("invalid identity config JSON: {error}"))
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
        .ok_or_else(|| "cannot resolve ~/.burd directory".to_string())?;
    fs::create_dir_all(dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;

    let private_key_path = dir.join("agent.key");
    let machine_id = format!("burd-{}", Uuid::new_v4());
    let private_key_placeholder = format!("burd-private-key-placeholder-{}", Uuid::new_v4());
    let public_key = format!("burd-public-key-placeholder-{}", Uuid::new_v4());

    fs::write(&private_key_path, private_key_placeholder).map_err(|error| {
        format!(
            "failed to write private key placeholder at {}: {error}",
            private_key_path.display()
        )
    })?;

    let config = AgentConfig {
        provider_id: None,
        machine_id,
        api_url: "https://api.burd.cloud".to_string(),
        preferred_provider: "ollama".to_string(),
        benchmark_profile: "auto".to_string(),
        telemetry_enabled: false,
        created_at: Utc::now().to_rfc3339(),
        public_key,
        private_key_path: private_key_path.display().to_string(),
    };

    let json = serde_json::to_string_pretty(&config)
        .map_err(|error| format!("failed to serialize identity config: {error}"))?;
    fs::write(&path, json)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;

    Ok(IdentityInitResult {
        config_path: path.display().to_string(),
        identity: config.public_identity(),
        created: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_identity_hides_private_key() {
        let config = AgentConfig {
            provider_id: Some("provider".to_string()),
            machine_id: "machine".to_string(),
            api_url: "https://api.example".to_string(),
            preferred_provider: "ollama".to_string(),
            benchmark_profile: "profile_12gb".to_string(),
            telemetry_enabled: false,
            created_at: "2026-06-08T00:00:00Z".to_string(),
            public_key: "pub".to_string(),
            private_key_path: "/secret".to_string(),
        };

        let json = serde_json::to_string(&config.public_identity()).unwrap();
        assert!(!json.contains("private_key_path"));
    }

    #[test]
    fn config_json_roundtrip() {
        let config = AgentConfig {
            provider_id: None,
            machine_id: "machine".to_string(),
            api_url: "https://api.example".to_string(),
            preferred_provider: "ollama".to_string(),
            benchmark_profile: "auto".to_string(),
            telemetry_enabled: true,
            created_at: "2026-06-08T00:00:00Z".to_string(),
            public_key: "pub".to_string(),
            private_key_path: "/secret".to_string(),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.machine_id, "machine");
    }
}
