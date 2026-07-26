use burd_protocol::default_state_dir;
use chrono::Utc;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const REMOTE_SESSION_LOCK_SCHEMA_VERSION: &str = "1";
const REMOTE_SESSION_LOCK_FILE: &str = "remote-session.lock";

#[derive(Debug, Serialize)]
struct RemoteSessionLockMetadata {
    schema_version: &'static str,
    pid: u32,
    acquired_at: String,
}

#[derive(Debug)]
pub(crate) struct RemoteSessionInstanceLock {
    _file: File,
}

impl RemoteSessionInstanceLock {
    pub(crate) fn acquire() -> Result<Self, String> {
        Self::acquire_at(default_state_dir().join(REMOTE_SESSION_LOCK_FILE))
    }

    fn acquire_at(path: PathBuf) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "remote session lock path has no parent".to_string())?;
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
                    "failed to open remote session lock at {}: {error}",
                    path.display()
                )
            })?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "another remote-session connect process is already using {}",
                    parent.display()
                ));
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(format!(
                    "failed to acquire remote session lock at {}: {error}",
                    path.display()
                ));
            }
        }

        write_lock_metadata(&mut file, &path)?;
        Ok(Self { _file: file })
    }
}

fn reject_unsafe_lock_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "remote session lock path {} must not be a symbolic link",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "remote session lock path {} is not a regular file",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect remote session lock at {}: {error}",
            path.display()
        )),
    }
}

fn write_lock_metadata(file: &mut File, path: &Path) -> Result<(), String> {
    let metadata = RemoteSessionLockMetadata {
        schema_version: REMOTE_SESSION_LOCK_SCHEMA_VERSION,
        pid: std::process::id(),
        acquired_at: Utc::now().to_rfc3339(),
    };
    let bytes = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| format!("failed to serialize remote session lock: {error}"))?;
    file.set_len(0).map_err(|error| {
        format!(
            "failed to truncate remote session lock at {}: {error}",
            path.display()
        )
    })?;
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        format!(
            "failed to seek remote session lock at {}: {error}",
            path.display()
        )
    })?;
    file.write_all(&bytes).map_err(|error| {
        format!(
            "failed to write remote session lock at {}: {error}",
            path.display()
        )
    })?;
    file.sync_data().map_err(|error| {
        format!(
            "failed to sync remote session lock at {}: {error}",
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
    fn lock_excludes_a_second_process_handle_and_releases_on_drop() {
        let root = test_root("exclusive");
        let path = root.join(REMOTE_SESSION_LOCK_FILE);
        let first = RemoteSessionInstanceLock::acquire_at(path.clone()).unwrap();

        let error = RemoteSessionInstanceLock::acquire_at(path.clone()).unwrap_err();
        assert!(error.contains("another remote-session connect process"));

        drop(first);
        let reacquired = RemoteSessionInstanceLock::acquire_at(path.clone()).unwrap();
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
        let path = root.join(REMOTE_SESSION_LOCK_FILE);
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, "stale metadata is overwritten").unwrap();

        let mut lock = RemoteSessionInstanceLock::acquire_at(path.clone()).unwrap();
        lock._file.seek(SeekFrom::Start(0)).unwrap();
        let mut raw = String::new();
        lock._file.read_to_string(&mut raw).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["schema_version"], REMOTE_SESSION_LOCK_SCHEMA_VERSION);
        assert_eq!(value["pid"], std::process::id());
        assert!(value["acquired_at"].as_str().is_some());
        assert!(raw.len() < 512);
        for forbidden in [
            "authorization",
            "credential",
            "private_key",
            "resume_token",
            "signature",
            "token",
        ] {
            assert!(!raw.to_ascii_lowercase().contains(forbidden));
        }

        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_file_lock_path_is_rejected() {
        let root = test_root("directory");
        let path = root.join(REMOTE_SESSION_LOCK_FILE);
        fs::create_dir_all(&path).unwrap();

        let error = RemoteSessionInstanceLock::acquire_at(path).unwrap_err();
        assert!(error.contains("is not a regular file"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn process_lock_is_released_when_holder_is_terminated() {
        if let Some(path) = std::env::var_os("BURD_TEST_REMOTE_SESSION_LOCK_PATH") {
            let _lock = RemoteSessionInstanceLock::acquire_at(PathBuf::from(path)).unwrap();
            println!("BURD_TEST_REMOTE_SESSION_LOCKED");
            std::io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        let root = test_root("process");
        let path = root.join(REMOTE_SESSION_LOCK_FILE);
        let executable = std::env::current_exe().unwrap();
        let mut child = Command::new(executable)
            .args([
                "--exact",
                "instance_lock::tests::process_lock_is_released_when_holder_is_terminated",
                "--nocapture",
            ])
            .env("BURD_TEST_REMOTE_SESSION_LOCK_PATH", &path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut ready = false;
        while reader.read_line(&mut line).unwrap() != 0 {
            if line.contains("BURD_TEST_REMOTE_SESSION_LOCKED") {
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

        let error = RemoteSessionInstanceLock::acquire_at(path.clone()).unwrap_err();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(error.contains("another remote-session connect process"));

        let reacquired = RemoteSessionInstanceLock::acquire_at(path.clone()).unwrap();
        drop(reacquired);
        fs::remove_dir_all(root).unwrap();
    }

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "burd-agent-instance-lock-{label}-{}-{nonce}",
            std::process::id()
        ))
    }
}
