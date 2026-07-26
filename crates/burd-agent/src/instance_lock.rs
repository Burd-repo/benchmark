use burd_protocol::default_state_dir;
use chrono::Utc;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const AGENT_STATE_LOCK_SCHEMA_VERSION: &str = "2";
const AGENT_STATE_LOCK_FILE: &str = "remote-session.lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStateLockOperation {
    RemoteSessionConnect,
    IdentityInit,
    IdentityMigrate,
    IdentityRotateKey,
    EnrollmentEnroll,
    EnrollmentRefreshCredential,
    ApiTokenCreate,
    ApiTokenRotate,
}

impl AgentStateLockOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::RemoteSessionConnect => "remote_session_connect",
            Self::IdentityInit => "identity_init",
            Self::IdentityMigrate => "identity_migrate",
            Self::IdentityRotateKey => "identity_rotate_key",
            Self::EnrollmentEnroll => "enrollment_enroll",
            Self::EnrollmentRefreshCredential => "enrollment_refresh_credential",
            Self::ApiTokenCreate => "api_token_create",
            Self::ApiTokenRotate => "api_token_rotate",
        }
    }
}

#[derive(Debug, Serialize)]
struct AgentStateLockMetadata {
    schema_version: &'static str,
    operation: &'static str,
    pid: u32,
    acquired_at: String,
}

#[derive(Debug)]
#[must_use = "the Agent state lock must be held for the protected operation"]
pub struct AgentStateLock {
    _file: File,
}

impl AgentStateLock {
    pub fn acquire(operation: AgentStateLockOperation) -> Result<Self, String> {
        Self::acquire_at(default_state_dir().join(AGENT_STATE_LOCK_FILE), operation)
    }

    fn acquire_at(path: PathBuf, operation: AgentStateLockOperation) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Agent state lock path has no parent".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        reject_unsafe_lock_path(&path)?;

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                format!(
                    "failed to open Agent state lock at {}: {error}",
                    path.display()
                )
            })?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "cannot start {}: another Agent process is using {}; wait for maintenance to finish or stop remote-session connect",
                    operation.as_str(),
                    parent.display(),
                ));
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(format!(
                    "failed to acquire Agent state lock at {}: {error}",
                    path.display()
                ));
            }
        }

        write_lock_metadata(&mut file, &path, operation)?;
        Ok(Self { _file: file })
    }
}

fn reject_unsafe_lock_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "Agent state lock path {} must not be a symbolic link",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "Agent state lock path {} is not a regular file",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect Agent state lock at {}: {error}",
            path.display()
        )),
    }
}

fn write_lock_metadata(
    file: &mut File,
    path: &Path,
    operation: AgentStateLockOperation,
) -> Result<(), String> {
    let metadata = AgentStateLockMetadata {
        schema_version: AGENT_STATE_LOCK_SCHEMA_VERSION,
        operation: operation.as_str(),
        pid: std::process::id(),
        acquired_at: Utc::now().to_rfc3339(),
    };
    let bytes = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("failed to serialize Agent state lock: {error}"))?;
    file.set_len(0).map_err(|error| {
        format!(
            "failed to truncate Agent state lock at {}: {error}",
            path.display()
        )
    })?;
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        format!(
            "failed to seek Agent state lock at {}: {error}",
            path.display()
        )
    })?;
    file.write_all(&bytes).map_err(|error| {
        format!(
            "failed to write Agent state lock at {}: {error}",
            path.display()
        )
    })?;
    file.sync_data().map_err(|error| {
        format!(
            "failed to sync Agent state lock at {}: {error}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Command, Stdio};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn lock_excludes_a_second_operation_and_releases_on_drop() {
        let root = test_root("exclusive");
        let path = root.join(AGENT_STATE_LOCK_FILE);
        let first =
            AgentStateLock::acquire_at(path.clone(), AgentStateLockOperation::RemoteSessionConnect)
                .unwrap();

        let error =
            AgentStateLock::acquire_at(path.clone(), AgentStateLockOperation::IdentityRotateKey)
                .unwrap_err();
        assert!(error.contains("cannot start identity_rotate_key"));
        assert!(error.contains("another Agent process is using"));

        drop(first);
        let reacquired =
            AgentStateLock::acquire_at(path.clone(), AgentStateLockOperation::IdentityRotateKey)
                .unwrap();
        drop(reacquired);
        assert!(
            path.is_file(),
            "lock file intentionally remains after release"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lock_metadata_is_bounded_and_contains_no_credentials() {
        let root = test_root("metadata");
        let path = root.join(AGENT_STATE_LOCK_FILE);
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, "stale metadata is overwritten").unwrap();

        let mut lock =
            AgentStateLock::acquire_at(path.clone(), AgentStateLockOperation::ApiTokenRotate)
                .unwrap();
        lock._file.seek(SeekFrom::Start(0)).unwrap();
        let mut raw = String::new();
        lock._file.read_to_string(&mut raw).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["schema_version"], AGENT_STATE_LOCK_SCHEMA_VERSION);
        assert_eq!(value["operation"], "api_token_rotate");
        assert_eq!(value["pid"], std::process::id());
        assert!(value["acquired_at"].as_str().is_some());
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert!(raw.len() < 512);
        for forbidden in [
            "authorization",
            "bearer",
            "private_key",
            "resume_token",
            "secret",
            "signature",
        ] {
            assert!(!raw.to_ascii_lowercase().contains(forbidden));
        }

        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_file_lock_path_is_rejected() {
        let root = test_root("directory");
        let path = root.join(AGENT_STATE_LOCK_FILE);
        fs::create_dir_all(&path).unwrap();

        let error = AgentStateLock::acquire_at(path, AgentStateLockOperation::EnrollmentEnroll)
            .unwrap_err();
        assert!(error.contains("is not a regular file"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn process_lock_is_released_when_holder_is_terminated() {
        if let Some(path) = std::env::var_os("BURD_TEST_AGENT_STATE_LOCK_PATH") {
            let _lock = AgentStateLock::acquire_at(
                PathBuf::from(path),
                AgentStateLockOperation::RemoteSessionConnect,
            )
            .unwrap();
            println!("BURD_TEST_AGENT_STATE_LOCKED");
            std::io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_secs(30));
            return;
        }

        let root = test_root("process");
        let path = root.join(AGENT_STATE_LOCK_FILE);
        let executable = std::env::current_exe().unwrap();
        let mut child = Command::new(executable)
            .args([
                "--exact",
                "instance_lock::tests::process_lock_is_released_when_holder_is_terminated",
                "--nocapture",
            ])
            .env("BURD_TEST_AGENT_STATE_LOCK_PATH", &path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut ready = false;
        while reader.read_line(&mut line).unwrap() != 0 {
            if line.contains("BURD_TEST_AGENT_STATE_LOCKED") {
                ready = true;
                break;
            }
            line.clear();
        }
        if !ready {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "child did not acquire the lock: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let error = AgentStateLock::acquire_at(
            path.clone(),
            AgentStateLockOperation::EnrollmentRefreshCredential,
        )
        .unwrap_err();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(error.contains("cannot start enrollment_refresh_credential"));

        let reacquired = AgentStateLock::acquire_at(
            path.clone(),
            AgentStateLockOperation::EnrollmentRefreshCredential,
        )
        .unwrap();
        drop(reacquired);
        fs::remove_dir_all(root).unwrap();
    }

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "burd-agent-state-lock-{label}-{}-{nonce}",
            std::process::id()
        ))
    }
}
