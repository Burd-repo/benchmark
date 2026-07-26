#![cfg(windows)]

use burd_protocol::write_json_atomic;
use serde_json::json;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, ERROR_SHARING_VIOLATION,
};
use windows_sys::Win32::Security::Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetFileSecurityW, GetSecurityDescriptorControl,
    GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
    UNPROTECTED_DACL_SECURITY_INFORMATION,
};

#[test]
fn atomic_replacement_preserves_a_protected_dacl() {
    let root = test_root();
    let path = root.join("state.json");
    write_json_atomic(&path, &json!({"version": 1})).unwrap();

    let mut original = read_dacl(&path).unwrap();
    apply_dacl(
        &path,
        &mut original.descriptor,
        PROTECTED_DACL_SECURITY_INFORMATION,
    )
    .unwrap();
    let before = read_dacl(&path).unwrap();
    assert_eq!(before.protection, PROTECTED_DACL_SECURITY_INFORMATION);

    write_json_atomic(&path, &json!({"version": 2})).unwrap();

    assert_eq!(read_dacl(&path).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[derive(Debug, Eq, PartialEq)]
struct Dacl {
    descriptor: Vec<u8>,
    protection: u32,
}

fn read_dacl(path: &Path) -> std::io::Result<Dacl> {
    let path_wide = wide_path(path);
    let mut length = 0_u32;
    let first_result = unsafe {
        GetFileSecurityW(
            path_wide.as_ptr(),
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut length,
        )
    };
    if first_result == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error().map(|code| code as u32) != Some(ERROR_INSUFFICIENT_BUFFER)
            || length == 0
        {
            return Err(error);
        }
    }

    let mut descriptor = vec![0_u8; length as usize];
    let result = unsafe {
        GetFileSecurityW(
            path_wide.as_ptr(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }

    let mut control = 0_u16;
    let mut revision = 0_u32;
    let result = unsafe {
        GetSecurityDescriptorControl(descriptor.as_mut_ptr().cast(), &mut control, &mut revision)
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let protection = if control & SE_DACL_PROTECTED != 0 {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    };
    Ok(Dacl {
        descriptor,
        protection,
    })
}

fn apply_dacl(path: &Path, descriptor: &mut [u8], protection: u32) -> std::io::Result<()> {
    let mut dacl_present = 0;
    let mut dacl = std::ptr::null_mut();
    let mut dacl_defaulted = 0;
    let result = unsafe {
        GetSecurityDescriptorDacl(
            descriptor.as_mut_ptr().cast(),
            &mut dacl_present,
            &mut dacl,
            &mut dacl_defaulted,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }

    let path_wide = wide_path(path);
    for attempt in 0..320 {
        let status = unsafe {
            SetNamedSecurityInfoW(
                path_wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | protection,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                if dacl_present != 0 {
                    dacl
                } else {
                    std::ptr::null_mut()
                },
                std::ptr::null_mut(),
            )
        };
        if status == 0 {
            return Ok(());
        }
        let retryable = status == ERROR_ACCESS_DENIED || status == ERROR_SHARING_VIOLATION;
        if !retryable || attempt == 319 {
            return Err(std::io::Error::from_raw_os_error(status as i32));
        }
        let delay_ms = 1_u64 << attempt.min(4);
        std::thread::sleep(Duration::from_millis(delay_ms));
    }
    unreachable!("bounded Windows test DACL loop always returns")
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn test_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "burd-protocol-windows-dacl-{}-{nonce}",
        std::process::id()
    ))
}
