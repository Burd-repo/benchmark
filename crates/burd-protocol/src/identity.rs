use crate::signature::{
    KEY_ALGORITHM, encode_base64, generate_keypair, sha256_hex, sign_message, verify_message,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MIGRATED_STATE_FILES: [&str; 7] = [
    "latest-report.json",
    "latest-signed-report.json",
    "latest-challenge-response.json",
    "benchmark-history.json",
    "uptime.json",
    "actions.json",
    "logs.json",
];

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatePaths {
    pub state_dir: String,
    pub config_path: String,
    pub source: String,
    pub consistent: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityMigrationResult {
    pub state_dir: String,
    pub config_path: String,
    pub source_config_path: String,
    pub backup_dir: Option<String>,
    pub migrated: bool,
    pub identity: AgentIdentityPublic,
    pub warnings: Vec<String>,
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
    resolved_state_paths().0
}

pub fn default_config_path() -> PathBuf {
    resolved_state_paths().1
}

pub fn agent_state_paths() -> AgentStatePaths {
    let (state_dir, config_path, source, warnings) = resolved_state_paths();
    AgentStatePaths {
        state_dir: state_dir.display().to_string(),
        config_path: config_path.display().to_string(),
        source,
        consistent: warnings.is_empty(),
        warnings,
    }
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

    let config = create_fresh_identity(&path)?;

    Ok(IdentityInitResult {
        config_path: path.display().to_string(),
        identity: config.public_identity(),
        created: true,
    })
}

pub fn migrate_identity(
    source: Option<&Path>,
    confirm: bool,
) -> Result<IdentityMigrationResult, String> {
    if !confirm {
        return Err("identity migration requires --confirm".to_string());
    }

    let target_state_dir = default_state_dir();
    let target_config_path = default_config_path();
    let source_config_path = source
        .map(source_config_path)
        .unwrap_or_else(|| target_config_path.clone());
    let source_state_dir = source_config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let same_state = source_state_dir == target_state_dir;
    let mut warnings = Vec::new();

    if !source_config_path.exists() {
        if source.is_some() {
            return Err(format!(
                "source identity config not found at {}",
                source_config_path.display()
            ));
        }
        let config = create_fresh_identity(&target_config_path)?;
        warnings.push("No previous identity existed; initialized a fresh identity.".to_string());
        return Ok(IdentityMigrationResult {
            state_dir: target_state_dir.display().to_string(),
            config_path: target_config_path.display().to_string(),
            source_config_path: source_config_path.display().to_string(),
            backup_dir: None,
            migrated: true,
            identity: config.public_identity(),
            warnings,
        });
    }

    let raw = fs::read_to_string(&source_config_path).map_err(|error| {
        format!(
            "failed to read source identity config at {}: {error}",
            source_config_path.display()
        )
    })?;
    let parsed = serde_json::from_str::<AgentConfig>(&raw);

    let mut config = match parsed {
        Ok(config) => config,
        Err(error) if source.is_none() => {
            let backup_dir = backup_existing_state(&target_state_dir)?;
            warnings.push(format!(
                "Previous identity config was invalid and was replaced after backup: {error}"
            ));
            let config = create_fresh_identity(&target_config_path)?;
            return Ok(IdentityMigrationResult {
                state_dir: target_state_dir.display().to_string(),
                config_path: target_config_path.display().to_string(),
                source_config_path: source_config_path.display().to_string(),
                backup_dir: backup_dir.map(|path| path.display().to_string()),
                migrated: true,
                identity: config.public_identity(),
                warnings,
            });
        }
        Err(error) => {
            return Err(format!(
                "source identity config at {} is invalid: {error}",
                source_config_path.display()
            ));
        }
    };

    let private_key = match load_private_key(&config).and_then(|key| {
        validate_keypair(&config, &key)?;
        Ok(key)
    }) {
        Ok(key) => key,
        Err(error) if source.is_none() => {
            let backup_dir = backup_existing_state(&target_state_dir)?;
            warnings.push(format!(
                "Previous signing key was unavailable or invalid and was replaced after backup: {error}"
            ));
            if contains_legacy_secret_fields(&raw) {
                warnings.push(
                    "Legacy secret fields were removed from agent.json; the original remains in the backup."
                        .to_string(),
                );
            }
            let key = replace_identity_key(&mut config, &target_state_dir)?;
            write_private_key(Path::new(&config.private_key_path), &key)?;
            write_config(&target_config_path, &config)?;
            return Ok(IdentityMigrationResult {
                state_dir: target_state_dir.display().to_string(),
                config_path: target_config_path.display().to_string(),
                source_config_path: source_config_path.display().to_string(),
                backup_dir: backup_dir.map(|path| path.display().to_string()),
                migrated: true,
                identity: config.public_identity(),
                warnings,
            });
        }
        Err(error) => return Err(error),
    };
    let backup_dir = backup_existing_state(&target_state_dir)?;
    fs::create_dir_all(&target_state_dir).map_err(|error| {
        format!(
            "failed to create target state directory {}: {error}",
            target_state_dir.display()
        )
    })?;
    let target_private_key_path = target_state_dir.join("agent.key");
    write_private_key(&target_private_key_path, &private_key)?;
    config.private_key_path = target_private_key_path.display().to_string();
    write_config(&target_config_path, &config)?;

    if !same_state {
        copy_public_state_files(&source_state_dir, &target_state_dir)?;
    } else {
        warnings
            .push("Identity config was normalized in its existing state directory.".to_string());
    }
    if contains_legacy_secret_fields(&raw) {
        warnings.push(
            "Legacy secret fields were removed from agent.json; the original remains in the backup."
                .to_string(),
        );
    }

    Ok(IdentityMigrationResult {
        state_dir: target_state_dir.display().to_string(),
        config_path: target_config_path.display().to_string(),
        source_config_path: source_config_path.display().to_string(),
        backup_dir: backup_dir.map(|path| path.display().to_string()),
        migrated: true,
        identity: config.public_identity(),
        warnings,
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
    if default_state_dir().join("remote-enrollment.json").exists() {
        return Err(
            "local-only key rotation is blocked for an enrolled device; use the control-plane key rotation protocol"
                .to_string(),
        );
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

fn resolved_state_paths() -> (PathBuf, PathBuf, String, Vec<String>) {
    resolve_state_paths_from(
        non_empty_env("BURD_AGENT_HOME"),
        non_empty_env("BURD_AGENT_CONFIG"),
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")),
    )
}

fn resolve_state_paths_from(
    agent_home: Option<PathBuf>,
    agent_config: Option<PathBuf>,
    home_dir: PathBuf,
) -> (PathBuf, PathBuf, String, Vec<String>) {
    if let Some(config_path) = agent_config {
        let state_dir = config_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let mut warnings = Vec::new();
        if let Some(agent_home) = agent_home
            && agent_home != state_dir
        {
            warnings.push(format!(
                "BURD_AGENT_CONFIG takes precedence; BURD_AGENT_HOME={} is ignored to prevent split state.",
                agent_home.display()
            ));
        }
        return (
            state_dir,
            config_path,
            "burd_agent_config".to_string(),
            warnings,
        );
    }

    if let Some(state_dir) = agent_home {
        return (
            state_dir.clone(),
            state_dir.join("agent.json"),
            "burd_agent_home".to_string(),
            Vec::new(),
        );
    }

    let state_dir = home_dir.join(".burd");
    (
        state_dir.clone(),
        state_dir.join("agent.json"),
        "default_home".to_string(),
        Vec::new(),
    )
}

fn non_empty_env(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

fn source_config_path(source: &Path) -> PathBuf {
    if source.is_dir() {
        source.join("agent.json")
    } else {
        source.to_path_buf()
    }
}

fn create_fresh_identity(path: &Path) -> Result<AgentConfig, String> {
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
    write_config(path, &config)?;
    Ok(config)
}

fn validate_keypair(config: &AgentConfig, private_key: &PrivateKeyFile) -> Result<(), String> {
    let message = b"burd-identity-migration-validation";
    let signature = sign_message(&private_key.secret_key_base64, message)?;
    if verify_message(&config.public_key, message, &signature)? {
        Ok(())
    } else {
        Err("source identity public key does not match its private key".to_string())
    }
}

fn replace_identity_key(
    config: &mut AgentConfig,
    state_dir: &Path,
) -> Result<PrivateKeyFile, String> {
    let keypair = generate_keypair()?;
    let private_key = PrivateKeyFile {
        key_algorithm: KEY_ALGORITHM.to_string(),
        secret_key_base64: keypair.secret_key_base64,
        created_at: Utc::now().to_rfc3339(),
    };
    config.public_key = keypair.public_key_base64;
    config.key_algorithm = KEY_ALGORITHM.to_string();
    config.private_key_path = state_dir.join("agent.key").display().to_string();
    Ok(private_key)
}

fn backup_existing_state(state_dir: &Path) -> Result<Option<PathBuf>, String> {
    if !state_dir.exists() {
        return Ok(None);
    }
    let files = fs::read_dir(state_dir)
        .map_err(|error| format!("failed to inspect {}: {error}", state_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Ok(None);
    }

    let backup_dir = state_dir.join(format!(
        "migration-backup-{}",
        Utc::now().timestamp_millis()
    ));
    fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("failed to create {}: {error}", backup_dir.display()))?;
    for entry in files {
        let target = backup_dir.join(entry.file_name());
        fs::copy(entry.path(), &target).map_err(|error| {
            format!(
                "failed to back up {} to {}: {error}",
                entry.path().display(),
                target.display()
            )
        })?;
    }
    Ok(Some(backup_dir))
}

fn copy_public_state_files(source_dir: &Path, target_dir: &Path) -> Result<(), String> {
    for name in MIGRATED_STATE_FILES {
        let source = source_dir.join(name);
        if !source.exists() {
            continue;
        }
        let target = target_dir.join(name);
        fs::copy(&source, &target).map_err(|error| {
            format!(
                "failed to migrate {} to {}: {error}",
                source.display(),
                target.display()
            )
        })?;
    }
    Ok(())
}

fn contains_legacy_secret_fields(raw: &str) -> bool {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|map| {
            ["private_key", "secret_key_base64", "api_token"]
                .iter()
                .any(|key| map.contains_key(*key))
        })
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
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

    #[test]
    fn config_override_is_the_canonical_state_directory() {
        let (state_dir, config_path, source, warnings) = resolve_state_paths_from(
            Some(PathBuf::from("ignored-home")),
            Some(PathBuf::from("canonical").join("agent.json")),
            PathBuf::from("home"),
        );

        assert_eq!(state_dir, PathBuf::from("canonical"));
        assert_eq!(config_path, PathBuf::from("canonical").join("agent.json"));
        assert_eq!(source, "burd_agent_config");
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn home_override_keeps_config_and_state_together() {
        let (state_dir, config_path, source, warnings) =
            resolve_state_paths_from(Some(PathBuf::from("state")), None, PathBuf::from("home"));

        assert_eq!(state_dir, PathBuf::from("state"));
        assert_eq!(config_path, PathBuf::from("state").join("agent.json"));
        assert_eq!(source, "burd_agent_home");
        assert!(warnings.is_empty());
    }

    #[test]
    fn migration_imports_identity_and_persisted_evidence_with_backup() {
        let _guard = env_lock();
        let root = std::env::temp_dir().join(format!("burd-protocol-migrate-{}", Uuid::new_v4()));
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        let source_config = source.join("agent.json");
        let source_identity = create_fresh_identity(&source_config).unwrap();
        fs::write(source.join("benchmark-history.json"), "[]").unwrap();
        fs::write(target.join("old-state.json"), r#"{"legacy":true}"#).unwrap();
        let env = TestStateEnv::new(&target);

        let result = migrate_identity(Some(&source), true).unwrap();

        assert_eq!(result.identity.provider_id, source_identity.provider_id);
        assert!(target.join("agent.json").exists());
        assert!(target.join("agent.key").exists());
        assert!(target.join("benchmark-history.json").exists());
        let backup = PathBuf::from(result.backup_dir.unwrap());
        assert!(backup.join("old-state.json").exists());
        validate_keypair(
            &load_identity().unwrap(),
            &load_private_key(&load_identity().unwrap()).unwrap(),
        )
        .unwrap();

        drop(env);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migration_repairs_invalid_legacy_config_without_leaking_secret_fields() {
        let _guard = env_lock();
        let root = std::env::temp_dir().join(format!("burd-protocol-repair-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("agent.json"),
            r#"{"provider_id":null,"private_key":"legacy-secret"}"#,
        )
        .unwrap();
        let env = TestStateEnv::new(&root);

        let result = migrate_identity(None, true).unwrap();

        assert!(result.migrated);
        assert!(load_identity().is_ok());
        let normalized = fs::read_to_string(root.join("agent.json")).unwrap();
        assert!(!normalized.contains("legacy-secret"));
        assert!(!normalized.contains("\"private_key\""));
        let backup = PathBuf::from(result.backup_dir.unwrap());
        assert!(
            fs::read_to_string(backup.join("agent.json"))
                .unwrap()
                .contains("legacy-secret")
        );

        drop(env);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migration_repairs_missing_private_key_and_preserves_identity_ids() {
        let _guard = env_lock();
        let root =
            std::env::temp_dir().join(format!("burd-protocol-key-repair-{}", Uuid::new_v4()));
        let config_path = root.join("agent.json");
        let original = create_fresh_identity(&config_path).unwrap();
        fs::remove_file(root.join("agent.key")).unwrap();
        let env = TestStateEnv::new(&root);

        let result = migrate_identity(None, true).unwrap();

        assert_eq!(result.identity.provider_id, original.provider_id);
        assert_eq!(result.identity.machine_id, original.machine_id);
        let repaired = load_identity().unwrap();
        let repaired_key = load_private_key(&repaired).unwrap();
        validate_keypair(&repaired, &repaired_key).unwrap();
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("signing key"))
        );

        drop(env);
        let _ = fs::remove_dir_all(root);
    }

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    struct TestStateEnv {
        previous_home: Option<OsString>,
        previous_config: Option<OsString>,
    }

    impl TestStateEnv {
        fn new(state_dir: &Path) -> Self {
            let previous_home = std::env::var_os("BURD_AGENT_HOME");
            let previous_config = std::env::var_os("BURD_AGENT_CONFIG");
            // SAFETY: identity tests that mutate environment variables hold ENV_LOCK.
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

    impl Drop for TestStateEnv {
        fn drop(&mut self) {
            // SAFETY: identity tests that mutate environment variables hold ENV_LOCK.
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
