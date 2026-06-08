use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use sysinfo::Disks;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskBenchmarkOptions {
    pub directory: PathBuf,
    pub file_size_mb: u64,
}

impl Default for DiskBenchmarkOptions {
    fn default() -> Self {
        Self {
            directory: std::env::temp_dir(),
            file_size_mb: 32,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskBenchmarkReport {
    pub directory: String,
    pub free_space_gb: Option<f64>,
    pub sequential_read_mb_s: f64,
    pub sequential_write_mb_s: f64,
    pub temp_file_size_mb: u64,
    pub passed: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

pub fn run_disk_benchmark(options: DiskBenchmarkOptions) -> DiskBenchmarkReport {
    let dir = options.directory;
    let size_mb = options.file_size_mb.max(1);
    if let Err(error) = fs::create_dir_all(&dir) {
        return failed_disk_report(
            dir,
            size_mb,
            format!("failed to create test directory: {error}"),
        );
    }

    let path = dir.join(format!("burd-disk-bench-{}.tmp", std::process::id()));
    let result = run_disk_io(&path, size_mb);
    let _ = fs::remove_file(&path);

    match result {
        Ok((write_mb_s, read_mb_s)) => summarize_disk(dir, size_mb, write_mb_s, read_mb_s, vec![]),
        Err(error) => failed_disk_report(dir, size_mb, error),
    }
}

fn run_disk_io(path: &Path, size_mb: u64) -> Result<(f64, f64), String> {
    let buffer = vec![0xAB; 1024 * 1024];
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("failed to create temp file: {error}"))?;

    let write_start = Instant::now();
    for _ in 0..size_mb {
        file.write_all(&buffer)
            .map_err(|error| format!("failed to write temp file: {error}"))?;
    }
    file.sync_all()
        .map_err(|error| format!("failed to sync temp file: {error}"))?;
    drop(file);
    let write_secs = write_start.elapsed().as_secs_f64().max(0.001);

    let mut file =
        File::open(path).map_err(|error| format!("failed to read temp file: {error}"))?;
    let mut read_buffer = vec![0u8; 1024 * 1024];
    let read_start = Instant::now();
    loop {
        let bytes = file
            .read(&mut read_buffer)
            .map_err(|error| format!("failed to read temp file: {error}"))?;
        if bytes == 0 {
            break;
        }
    }
    let read_secs = read_start.elapsed().as_secs_f64().max(0.001);

    Ok((size_mb as f64 / write_secs, size_mb as f64 / read_secs))
}

pub fn summarize_disk(
    directory: PathBuf,
    size_mb: u64,
    write_mb_s: f64,
    read_mb_s: f64,
    errors: Vec<String>,
) -> DiskBenchmarkReport {
    let mut warnings = Vec::new();
    if write_mb_s < 50.0 {
        warnings.push("sequential write below 50 MB/s".to_string());
    }
    if read_mb_s < 50.0 {
        warnings.push("sequential read below 50 MB/s".to_string());
    }

    DiskBenchmarkReport {
        directory: directory.display().to_string(),
        free_space_gb: free_space_gb(&directory).map(round2),
        sequential_read_mb_s: round1(read_mb_s),
        sequential_write_mb_s: round1(write_mb_s),
        temp_file_size_mb: size_mb,
        passed: errors.is_empty() && read_mb_s >= 50.0 && write_mb_s >= 50.0,
        warnings,
        errors,
    }
}

fn failed_disk_report(directory: PathBuf, size_mb: u64, error: String) -> DiskBenchmarkReport {
    DiskBenchmarkReport {
        directory: directory.display().to_string(),
        free_space_gb: free_space_gb(&directory).map(round2),
        sequential_read_mb_s: 0.0,
        sequential_write_mb_s: 0.0,
        temp_file_size_mb: size_mb,
        passed: false,
        warnings: vec!["disk benchmark failed".to_string()],
        errors: vec![error],
    }
}

fn free_space_gb(directory: &Path) -> Option<f64> {
    let canonical = directory.canonicalize().ok()?;
    let disks = Disks::new_with_refreshed_list();
    disks
        .iter()
        .filter(|disk| canonical.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| disk.available_space() as f64 / 1_073_741_824.0)
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_summary_fails_slow_io() {
        let report = summarize_disk(PathBuf::from("."), 1, 10.0, 10.0, vec![]);
        assert!(!report.passed);
        assert!(!report.warnings.is_empty());
    }
}
