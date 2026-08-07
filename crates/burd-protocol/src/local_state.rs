use crate::signature::random_token;
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const TEMPORARY_FILE_ATTEMPTS: usize = 4;
#[cfg(windows)]
const WINDOWS_REPLACE_ATTEMPTS: usize = 20;

pub fn write_json_atomic<T>(path: &Path, value: &T) -> Result<(), String>
where
    T: Serialize + ?Sized,
{
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to serialize JSON for {}: {error}", path.display()))?;
    write_bytes_atomic(path, &bytes)
}

pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    validate_destination(path)?;
    let (temporary, mut file) = create_temporary_file(path)?;

    let result = (|| {
        file.write_all(bytes).map_err(|error| {
            format!(
                "failed to write temporary state file at {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync temporary state file at {}: {error}",
                temporary.display()
            )
        })?;
        drop(file);
        replace_file(&temporary, path).map_err(|error| {
            format!(
                "failed to atomically replace state file at {}: {error}",
                path.display()
            )
        })?;
        sync_parent_directory(parent).map_err(|error| {
            format!(
                "failed to sync state directory at {}: {error}",
                parent.display()
            )
        })
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn validate_destination(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing to replace symbolic-link state path {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "state path {} is not a regular file",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect state path {}: {error}",
            path.display()
        )),
    }
}

fn create_temporary_file(path: &Path) -> Result<(PathBuf, File), String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("state path {} has no file name", path.display()))?;
    let mut last_collision = None;

    for _ in 0..TEMPORARY_FILE_ATTEMPTS {
        let token = random_token("atomic_write")
            .map_err(|error| format!("failed to generate temporary state path: {error}"))?;
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".{token}.tmp"));
        let temporary = path.with_file_name(temporary_name);
        match create_private_file(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_collision = Some(error);
            }
            Err(error) => {
                return Err(format!(
                    "failed to create temporary state file for {}: {error}",
                    path.display()
                ));
            }
        }
    }

    Err(format!(
        "failed to create unique temporary state file for {}: {}",
        path.display(),
        last_collision
            .map(|error| error.to_string())
            .unwrap_or_else(|| "temporary path collision".to_string())
    ))
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(windows)]
fn create_private_file(path: &Path) -> std::io::Result<File> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows_sys::Win32::Storage::FileSystem::{CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL};

    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let descriptor_sddl = "D:P(A;;FA;;;OW)(A;;FA;;;SY)"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor_sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(std::io::Error::last_os_error());
    }

    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let handle = unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            GENERIC_WRITE,
            0,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    let result = if handle == INVALID_HANDLE_VALUE {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_handle(handle) })
    };
    unsafe {
        LocalFree(descriptor);
    }
    result
}

#[cfg(not(any(unix, windows)))]
fn create_private_file(path: &Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
}

#[cfg(windows)]
static WINDOWS_REPLACEMENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(windows)]
fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION};
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let _guard = WINDOWS_REPLACEMENT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temporary_wide = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    for attempt in 0..WINDOWS_REPLACE_ATTEMPTS {
        let result = unsafe {
            MoveFileExW(
                temporary_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result != 0 {
            return Ok(());
        }

        let error = std::io::Error::last_os_error();
        let retryable = error.raw_os_error().is_some_and(|code| {
            code as u32 == ERROR_ACCESS_DENIED || code as u32 == ERROR_SHARING_VIOLATION
        });
        if !retryable || attempt + 1 == WINDOWS_REPLACE_ATTEMPTS {
            return Err(error);
        }
        let delay_ms = 1_u64 << attempt.min(4);
        std::thread::sleep(Duration::from_millis(delay_ms));
    }
    unreachable!("bounded Windows replacement loop always returns")
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> std::io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serializer;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn atomic_write_replaces_existing_json_and_cleans_temporary_files() {
        let root = test_root("replace");
        let path = root.join("state.json");
        write_json_atomic(&path, &json!({"version": 1, "value": "old"})).unwrap();
        write_json_atomic(&path, &json!({"version": 2, "value": "new"})).unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["version"], 2);
        assert_eq!(value["value"], "new");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn serialization_failure_keeps_the_previous_file() {
        let root = test_root("serialize");
        let path = root.join("state.json");
        write_json_atomic(&path, &json!({"version": 1})).unwrap();

        let error = write_json_atomic(&path, &AlwaysFailsSerialization).unwrap_err();
        assert!(error.contains("failed to serialize JSON"));
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_readers_never_observe_partial_json() {
        let root = test_root("concurrent");
        let path = root.join("state.json");
        let first = json!({"version": 0, "payload": "a".repeat(32 * 1024)});
        write_json_atomic(&path, &first).unwrap();

        let done = Arc::new(AtomicBool::new(false));
        let reader_done = Arc::clone(&done);
        let reader_path = path.clone();
        let reader = std::thread::spawn(move || {
            while !reader_done.load(Ordering::Acquire) {
                let raw = fs::read_to_string(&reader_path).unwrap();
                let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
                assert!(value["version"].as_u64().is_some());
                assert_eq!(value["payload"].as_str().unwrap().len(), 32 * 1024);
            }
        });

        for version in 1..=64 {
            let value = json!({
                "version": version,
                "payload": if version % 2 == 0 {
                    "a".repeat(32 * 1024)
                } else {
                    "b".repeat(32 * 1024)
                },
            });
            write_json_atomic(&path, &value).unwrap();
        }
        done.store(true, Ordering::Release);
        reader.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_file_destination_is_rejected_without_temporary_files() {
        let root = test_root("non-file");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.json");
        fs::create_dir(&path).unwrap();

        let error = write_json_atomic(&path, &json!({"version": 1})).unwrap_err();

        assert!(error.contains("is not a regular file"));
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_destination_is_rejected_without_changing_its_target() {
        use std::os::unix::fs::symlink;

        let root = test_root("symlink");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target.json");
        let path = root.join("state.json");
        fs::write(&target, r#"{"version":1}"#).unwrap();
        symlink(&target, &path).unwrap();

        let error = write_json_atomic(&path, &json!({"version": 2})).unwrap_err();

        assert!(error.contains("refusing to replace symbolic-link state path"));
        assert_eq!(fs::read_to_string(&target).unwrap(), r#"{"version":1}"#);
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn new_and_replaced_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = test_root("permissions");
        let path = root.join("state.json");
        write_json_atomic(&path, &json!({"version": 1})).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_json_atomic(&path, &json!({"version": 2})).unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }

    struct AlwaysFailsSerialization;

    impl Serialize for AlwaysFailsSerialization {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Err(serde::ser::Error::custom("expected test failure"))
        }
    }

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "burd-protocol-atomic-state-{label}-{}-{nonce}",
            std::process::id()
        ))
    }
}
