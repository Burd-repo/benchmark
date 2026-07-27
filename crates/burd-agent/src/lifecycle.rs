use burd_protocol::{default_state_dir, write_json_atomic};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const LIFECYCLE_SCHEMA_VERSION: &str = "burd.agent.lifecycle.v1";
const LIFECYCLE_FILE: &str = "agent-lifecycle.json";
const LIFECYCLE_LOCK_FILE: &str = "agent-lifecycle.lock";
const MAX_LIFECYCLE_FILE_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecyclePhase {
    Starting,
    Connecting,
    Online,
    Degraded,
    Stopping,
    TerminalFailure,
    Stopped,
}

impl AgentLifecyclePhase {
    fn ready(self) -> bool {
        self == Self::Online
    }

    fn process_should_be_active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Connecting | Self::Online | Self::Degraded | Self::Stopping
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedLifecycleStatus {
    schema_version: String,
    phase: AgentLifecyclePhase,
    ready: bool,
    updated_at: String,
    pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentLifecycleStatus {
    pub schema_version: String,
    pub phase: AgentLifecyclePhase,
    pub ready: bool,
    pub process_active: bool,
    pub updated_at: Option<String>,
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_observed_phase: Option<AgentLifecyclePhase>,
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleReporter {
    path: PathBuf,
    pid: u32,
    phase: Arc<Mutex<AgentLifecyclePhase>>,
    _liveness: Arc<File>,
}

impl LifecycleReporter {
    pub(crate) fn start() -> Result<Self, String> {
        Self::start_at(lifecycle_path())
    }

    fn start_at(path: PathBuf) -> Result<Self, String> {
        let liveness = acquire_liveness_lock(&lifecycle_lock_path_for(&path))?;
        let reporter = Self {
            path,
            pid: std::process::id(),
            phase: Arc::new(Mutex::new(AgentLifecyclePhase::Starting)),
            _liveness: Arc::new(liveness),
        };
        reporter.write(AgentLifecyclePhase::Starting, None)?;
        Ok(reporter)
    }

    pub(crate) fn transition(
        &self,
        next: AgentLifecyclePhase,
        failure_kind: Option<&str>,
    ) -> Result<(), String> {
        validate_failure_kind(failure_kind)?;
        let mut current = self
            .phase
            .lock()
            .map_err(|_| "Agent lifecycle state lock is poisoned".to_string())?;
        if !transition_allowed(*current, next) {
            if matches!(
                *current,
                AgentLifecyclePhase::Stopping
                    | AgentLifecyclePhase::TerminalFailure
                    | AgentLifecyclePhase::Stopped
            ) {
                return Ok(());
            }
            return Err(format!(
                "invalid Agent lifecycle transition from {:?} to {:?}",
                *current, next
            ));
        }
        self.write(next, failure_kind)?;
        *current = next;
        Ok(())
    }

    pub(crate) fn phase(&self) -> Result<AgentLifecyclePhase, String> {
        self.phase
            .lock()
            .map(|phase| *phase)
            .map_err(|_| "Agent lifecycle state lock is poisoned".to_string())
    }

    fn write(&self, phase: AgentLifecyclePhase, failure_kind: Option<&str>) -> Result<(), String> {
        write_json_atomic(
            &self.path,
            &PersistedLifecycleStatus {
                schema_version: LIFECYCLE_SCHEMA_VERSION.to_string(),
                phase,
                ready: phase.ready(),
                updated_at: Utc::now().to_rfc3339(),
                pid: self.pid,
                failure_kind: failure_kind.map(str::to_string),
            },
        )
    }
}

pub fn lifecycle_path() -> PathBuf {
    default_state_dir().join(LIFECYCLE_FILE)
}

pub fn lifecycle_status() -> Result<AgentLifecycleStatus, String> {
    load_status_at(&lifecycle_path())
}

fn load_status_at(path: &Path) -> Result<AgentLifecycleStatus, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(stopped_status());
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect Agent lifecycle state at {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "Agent lifecycle state path {} must be a regular file",
            path.display()
        ));
    }
    if metadata.len() > MAX_LIFECYCLE_FILE_BYTES {
        return Err(format!(
            "Agent lifecycle state at {} exceeds {} bytes",
            path.display(),
            MAX_LIFECYCLE_FILE_BYTES
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "failed to read Agent lifecycle state at {}: {error}",
            path.display()
        )
    })?;
    if bytes.len() as u64 > MAX_LIFECYCLE_FILE_BYTES {
        return Err(format!(
            "Agent lifecycle state at {} exceeds {} bytes",
            path.display(),
            MAX_LIFECYCLE_FILE_BYTES
        ));
    }
    let persisted: PersistedLifecycleStatus = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid Agent lifecycle state at {}: {error}",
            path.display()
        )
    })?;
    validate_persisted(&persisted)?;
    let process_active = lifecycle_lock_is_held(&lifecycle_lock_path_for(path))?;
    Ok(effective_status(persisted, process_active))
}

fn lifecycle_lock_path_for(status_path: &Path) -> PathBuf {
    status_path.with_file_name(LIFECYCLE_LOCK_FILE)
}

fn acquire_liveness_lock(path: &Path) -> Result<File, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Agent lifecycle lock path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    reject_unsafe_lifecycle_lock(path)?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            format!(
                "failed to open Agent lifecycle lock at {}: {error}",
                path.display()
            )
        })?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(format!(
            "another foreground Agent owns the lifecycle lock at {}",
            path.display()
        )),
        Err(std::fs::TryLockError::Error(error)) => Err(format!(
            "failed to acquire Agent lifecycle lock at {}: {error}",
            path.display()
        )),
    }
}

fn lifecycle_lock_is_held(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => reject_unsafe_lifecycle_lock(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "failed to inspect Agent lifecycle lock at {}: {error}",
                path.display()
            ));
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            format!(
                "failed to open Agent lifecycle lock at {}: {error}",
                path.display()
            )
        })?;
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(error)) => Err(format!(
            "failed to inspect Agent lifecycle lock at {}: {error}",
            path.display()
        )),
    }
}

fn reject_unsafe_lifecycle_lock(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "Agent lifecycle lock path {} must not be a symbolic link",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "Agent lifecycle lock path {} is not a regular file",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect Agent lifecycle lock at {}: {error}",
            path.display()
        )),
    }
}

fn validate_persisted(status: &PersistedLifecycleStatus) -> Result<(), String> {
    if status.schema_version != LIFECYCLE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported Agent lifecycle schema {}",
            status.schema_version
        ));
    }
    if status.ready != status.phase.ready() {
        return Err("Agent lifecycle readiness does not match its phase".to_string());
    }
    validate_failure_kind(status.failure_kind.as_deref())
}

fn validate_failure_kind(value: Option<&str>) -> Result<(), String> {
    if value.is_some_and(|value| {
        value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }) {
        return Err(
            "Agent lifecycle failure_kind must be a lowercase token up to 64 bytes".to_string(),
        );
    }
    Ok(())
}

fn effective_status(
    persisted: PersistedLifecycleStatus,
    process_active: bool,
) -> AgentLifecycleStatus {
    if persisted.phase.process_should_be_active() && !process_active {
        return AgentLifecycleStatus {
            schema_version: persisted.schema_version,
            phase: AgentLifecyclePhase::Stopped,
            ready: false,
            process_active: false,
            updated_at: Some(persisted.updated_at),
            pid: Some(persisted.pid),
            failure_kind: Some("process_not_running".to_string()),
            last_observed_phase: Some(persisted.phase),
        };
    }
    AgentLifecycleStatus {
        schema_version: persisted.schema_version,
        phase: persisted.phase,
        ready: persisted.ready,
        process_active,
        updated_at: Some(persisted.updated_at),
        pid: Some(persisted.pid),
        failure_kind: persisted.failure_kind,
        last_observed_phase: None,
    }
}

fn stopped_status() -> AgentLifecycleStatus {
    AgentLifecycleStatus {
        schema_version: LIFECYCLE_SCHEMA_VERSION.to_string(),
        phase: AgentLifecyclePhase::Stopped,
        ready: false,
        process_active: false,
        updated_at: None,
        pid: None,
        failure_kind: None,
        last_observed_phase: None,
    }
}

fn transition_allowed(current: AgentLifecyclePhase, next: AgentLifecyclePhase) -> bool {
    use AgentLifecyclePhase::*;
    current == next
        || matches!(
            (current, next),
            (Starting, Connecting | Stopping | TerminalFailure)
                | (Connecting, Online | Degraded | Stopping | TerminalFailure)
                | (Online, Degraded | Stopping | TerminalFailure)
                | (Degraded, Connecting | Stopping | TerminalFailure)
                | (Stopping, Stopped | TerminalFailure)
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn lifecycle_transitions_are_explicit_and_redacted() {
        let root = test_root("transitions");
        let path = root.join(LIFECYCLE_FILE);
        let reporter = LifecycleReporter::start_at(path.clone()).unwrap();
        reporter
            .transition(AgentLifecyclePhase::Connecting, None)
            .unwrap();
        reporter
            .transition(AgentLifecyclePhase::Online, None)
            .unwrap();
        reporter
            .transition(AgentLifecyclePhase::Degraded, Some("connection_error"))
            .unwrap();
        reporter
            .transition(AgentLifecyclePhase::Stopping, None)
            .unwrap();
        reporter
            .transition(AgentLifecyclePhase::Stopped, None)
            .unwrap();

        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("\"phase\": \"stopped\""));
        for forbidden in ["credential", "resume_token", "private_key", "authorization"] {
            assert!(!text.to_ascii_lowercase().contains(forbidden));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lifecycle_liveness_lock_tracks_process_ownership() {
        let root = test_root("liveness");
        let path = root.join(LIFECYCLE_FILE);
        let reporter = LifecycleReporter::start_at(path.clone()).unwrap();
        let active = load_status_at(&path).unwrap();
        assert_eq!(active.phase, AgentLifecyclePhase::Starting);
        assert!(active.process_active);
        drop(reporter);

        let stopped = load_status_at(&path).unwrap();
        assert_eq!(stopped.phase, AgentLifecyclePhase::Stopped);
        assert!(!stopped.ready);
        assert!(!stopped.process_active);
        assert_eq!(
            stopped.last_observed_phase,
            Some(AgentLifecyclePhase::Starting)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_online_snapshot_never_reports_ready() {
        let persisted = PersistedLifecycleStatus {
            schema_version: LIFECYCLE_SCHEMA_VERSION.to_string(),
            phase: AgentLifecyclePhase::Online,
            ready: true,
            updated_at: Utc::now().to_rfc3339(),
            pid: 42,
            failure_kind: None,
        };
        let status = effective_status(persisted, false);
        assert_eq!(status.phase, AgentLifecyclePhase::Stopped);
        assert!(!status.ready);
        assert!(!status.process_active);
        assert_eq!(
            status.last_observed_phase,
            Some(AgentLifecyclePhase::Online)
        );
        assert_eq!(status.failure_kind.as_deref(), Some("process_not_running"));
    }

    #[test]
    fn terminal_phase_rejects_late_online_transition() {
        let root = test_root("terminal");
        let reporter = LifecycleReporter::start_at(root.join(LIFECYCLE_FILE)).unwrap();
        reporter
            .transition(AgentLifecyclePhase::TerminalFailure, Some("revoked"))
            .unwrap();
        reporter
            .transition(AgentLifecyclePhase::Online, None)
            .unwrap();
        assert_eq!(
            reporter.phase().unwrap(),
            AgentLifecyclePhase::TerminalFailure
        );
        let _ = fs::remove_dir_all(root);
    }

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "burd-agent-lifecycle-{label}-{}-{nonce}",
            std::process::id()
        ))
    }
}
