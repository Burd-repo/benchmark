use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const STAGING_DIRECTORY: &str = "/burd/staging";
const VOLUME_DIRECTORY: &str = "/burd/volume";
const BUFFER_BYTES: usize = 64 * 1024;

fn main() {
    if let Err(error) = run(env::args().skip(1)) {
        eprintln!("burd artifact helper failed: {error}");
        std::process::exit(1);
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<(), &'static str> {
    if !cfg!(target_os = "linux") {
        return Err("linux container required");
    }
    let _ = fs::remove_file("/burd/staging/.burd-placeholder");
    let operation = args.next().ok_or("operation required")?;
    match operation.as_str() {
        "import" | "export" => {
            let maximum_bytes = parse_number(args.next(), "maximum bytes required")?;
            let maximum_files = parse_number(args.next(), "maximum files required")?;
            if args.next().is_some() || maximum_files == 0 || maximum_files > 32 {
                return Err("invalid helper arguments");
            }
            if operation == "import" {
                copy_direct_files(
                    Path::new(STAGING_DIRECTORY),
                    Path::new(VOLUME_DIRECTORY),
                    maximum_bytes,
                    maximum_files,
                    0o444,
                )
            } else {
                copy_direct_files(
                    Path::new(VOLUME_DIRECTORY),
                    Path::new(STAGING_DIRECTORY),
                    maximum_bytes,
                    maximum_files,
                    0o600,
                )
            }
        }
        "roundtrip-test" => {
            if args.next().is_some() {
                return Err("invalid helper arguments");
            }
            roundtrip_test()
        }
        _ => Err("unsupported operation"),
    }
}

fn parse_number(value: Option<String>, missing: &'static str) -> Result<u64, &'static str> {
    value
        .ok_or(missing)?
        .parse::<u64>()
        .map_err(|_| "invalid helper limit")
}

fn copy_direct_files(
    source: &Path,
    destination: &Path,
    maximum_bytes: u64,
    maximum_files: u64,
    destination_mode: u32,
) -> Result<(), &'static str> {
    require_directory(source)?;
    require_directory(destination)?;
    let mut entries = fs::read_dir(source)
        .map_err(|_| "source listing failed")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "source listing failed")?;
    entries.retain(|entry| entry.file_name() != ".burd-placeholder");
    entries.sort_by_key(|entry| entry.file_name());
    if entries.len() as u64 > maximum_files {
        return Err("file count exceeded");
    }

    let mut total = 0_u64;
    for entry in entries {
        let source_path = entry.path();
        let metadata = fs::symlink_metadata(&source_path).map_err(|_| "source metadata failed")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("source must contain direct regular files only");
        }
        total = total
            .checked_add(metadata.len())
            .filter(|total| *total <= maximum_bytes)
            .ok_or("byte limit exceeded")?;
        let destination_path = destination.join(entry.file_name());
        if destination_path.parent() != Some(destination) {
            return Err("destination escaped");
        }
        copy_one_file(
            &source_path,
            &destination_path,
            metadata.len(),
            destination_mode,
        )?;
    }
    Ok(())
}

fn require_directory(path: &Path) -> Result<(), &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "directory metadata failed")?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err("unsafe helper directory")
    }
}

fn copy_one_file(
    source: &Path,
    destination: &Path,
    expected_bytes: u64,
    destination_mode: u32,
) -> Result<(), &'static str> {
    let mut input = File::open(source).map_err(|_| "source open failed")?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "destination create failed")?;
    let result = (|| {
        let mut copied = 0_u64;
        let mut buffer = [0_u8; BUFFER_BYTES];
        loop {
            let read = input.read(&mut buffer).map_err(|_| "source read failed")?;
            if read == 0 {
                break;
            }
            copied = copied
                .checked_add(read as u64)
                .filter(|copied| *copied <= expected_bytes)
                .ok_or("source changed during copy")?;
            output
                .write_all(&buffer[..read])
                .map_err(|_| "destination write failed")?;
        }
        if copied != expected_bytes {
            return Err("source changed during copy");
        }
        output.sync_all().map_err(|_| "destination sync failed")?;
        set_mode(destination, destination_mode)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), &'static str> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|_| "destination permissions failed")
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), &'static str> {
    Err("linux container required")
}

fn roundtrip_test() -> Result<(), &'static str> {
    let input_path = Path::new("/burd/input/input.bin");
    let output_path = Path::new("/burd/output/output.bin");
    if OpenOptions::new().write(true).open(input_path).is_ok() {
        return Err("input mount is writable");
    }
    let mut input = File::open(input_path).map_err(|_| "roundtrip input open failed")?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
        .map_err(|_| "roundtrip output create failed")?;
    output
        .write_all(b"burd:")
        .map_err(|_| "roundtrip output write failed")?;
    std::io::copy(&mut input, &mut output).map_err(|_| "roundtrip copy failed")?;
    output
        .sync_all()
        .map_err(|_| "roundtrip output sync failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_strict() {
        assert_eq!(parse_number(Some("1".to_string()), "missing"), Ok(1));
        assert_eq!(parse_number(Some("0".to_string()), "missing"), Ok(0));
        assert!(parse_number(Some("invalid".to_string()), "missing").is_err());
    }
}
