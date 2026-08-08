use burd_protocol::{GpuProcessTelemetry, GpuTelemetrySample};
use chrono::Utc;
use csv::{ReaderBuilder, Trim};
use std::collections::{BTreeMap, HashSet};
use std::process::Command;

pub const NVIDIA_SMI_COLLECTOR_VERSION: &str = "nvidia-smi-csv-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvidiaGpuInventoryDevice {
    pub gpu_index: u32,
    pub gpu_uuid: String,
    pub pci_vendor_id: Option<String>,
    pub pci_device_id: Option<String>,
    pub vram_total_mib: u64,
}

#[derive(Debug, Clone)]
pub struct NvidiaTelemetryCollection {
    pub collector: String,
    pub samples: Vec<GpuTelemetrySample>,
    pub inventory: Vec<NvidiaGpuInventoryDevice>,
    pub warnings: Vec<String>,
}

pub fn collect_nvidia_telemetry(
    first_sample_sequence: u64,
) -> Result<NvidiaTelemetryCollection, String> {
    let identity = required_query(&[
        "index",
        "uuid",
        "name",
        "pci.bus_id",
        "pci.device_id",
        "driver_version",
        "memory.total",
    ])?;
    let observed_at = Utc::now().to_rfc3339();
    let cuda_driver_version = detect_cuda_driver_version();
    let mut samples = BTreeMap::new();
    let mut inventory = Vec::new();
    let mut gpu_indices = HashSet::new();
    for row in identity {
        require_columns(&row, 7, "GPU identity")?;
        let gpu_index =
            parse_u32(&row[0]).ok_or_else(|| "GPU index is invalid or unavailable".to_string())?;
        if !gpu_indices.insert(gpu_index) {
            return Err("nvidia-smi returned duplicate GPU indices".to_string());
        }
        let uuid = required_value(&row[1], "GPU UUID")?;
        let (pci_vendor_id, pci_device_id) = parse_pci_device_id(&row[4]);
        let vram_total_mib = parse_required_u64(&row[6], "total VRAM")?;
        inventory.push(NvidiaGpuInventoryDevice {
            gpu_index,
            gpu_uuid: uuid.clone(),
            pci_vendor_id: pci_vendor_id.clone(),
            pci_device_id: pci_device_id.clone(),
            vram_total_mib,
        });
        if samples
            .insert(
                uuid.clone(),
                GpuTelemetrySample {
                    sample_sequence: 0,
                    observed_at: observed_at.clone(),
                    gpu_uuid: uuid,
                    gpu_name: required_value(&row[2], "GPU name")?,
                    pci_bus_id: required_value(&row[3], "PCI bus ID")?,
                    pci_vendor_id,
                    pci_device_id,
                    compute_capability: None,
                    driver_version: required_value(&row[5], "driver version")?,
                    cuda_driver_version: cuda_driver_version.clone(),
                    cuda_runtime_version: None,
                    vram_total_mib,
                    vram_used_mib: None,
                    vram_free_mib: None,
                    gpu_utilization_percent: None,
                    memory_utilization_percent: None,
                    temperature_celsius: None,
                    power_draw_watts: None,
                    power_limit_watts: None,
                    graphics_clock_mhz: None,
                    sm_clock_mhz: None,
                    memory_clock_mhz: None,
                    performance_state: None,
                    throttle_reasons: vec![],
                    ecc_corrected_errors: None,
                    ecc_uncorrected_errors: None,
                    processes: vec![],
                    container_id: None,
                    job_id: None,
                },
            )
            .is_some()
        {
            return Err("nvidia-smi returned duplicate GPU UUIDs".to_string());
        }
    }
    if samples.is_empty() {
        return Err("nvidia-smi returned no NVIDIA GPUs".to_string());
    }

    let mut warnings = Vec::new();
    apply_optional_query(
        &mut samples,
        &[
            "uuid",
            "memory.used",
            "memory.free",
            "utilization.gpu",
            "utilization.memory",
        ],
        "memory/utilization",
        &mut warnings,
        |sample, row| {
            sample.vram_used_mib = parse_optional(row.get(1), parse_u64);
            sample.vram_free_mib = parse_optional(row.get(2), parse_u64);
            sample.gpu_utilization_percent = parse_optional(row.get(3), parse_percent);
            sample.memory_utilization_percent = parse_optional(row.get(4), parse_percent);
        },
    );
    apply_optional_query(
        &mut samples,
        &["uuid", "temperature.gpu", "power.draw", "power.limit"],
        "thermal/power",
        &mut warnings,
        |sample, row| {
            sample.temperature_celsius = parse_optional(row.get(1), parse_f64);
            sample.power_draw_watts = parse_optional(row.get(2), parse_nonnegative_f64);
            sample.power_limit_watts = parse_optional(row.get(3), parse_nonnegative_f64);
        },
    );
    apply_optional_query(
        &mut samples,
        &[
            "uuid",
            "clocks.current.graphics",
            "clocks.current.sm",
            "clocks.current.memory",
            "pstate",
        ],
        "clocks",
        &mut warnings,
        |sample, row| {
            sample.graphics_clock_mhz = parse_optional(row.get(1), parse_u32);
            sample.sm_clock_mhz = parse_optional(row.get(2), parse_u32);
            sample.memory_clock_mhz = parse_optional(row.get(3), parse_u32);
            sample.performance_state = row.get(4).and_then(|value| optional_text(value));
        },
    );
    apply_optional_query(
        &mut samples,
        &["uuid", "compute_cap"],
        "compute capability",
        &mut warnings,
        |sample, row| {
            sample.compute_capability = row.get(1).and_then(|value| optional_text(value));
        },
    );
    apply_optional_query(
        &mut samples,
        &[
            "uuid",
            "ecc.errors.corrected.volatile.total",
            "ecc.errors.uncorrected.volatile.total",
        ],
        "ECC",
        &mut warnings,
        |sample, row| {
            sample.ecc_corrected_errors = parse_optional(row.get(1), parse_u64);
            sample.ecc_uncorrected_errors = parse_optional(row.get(2), parse_u64);
        },
    );
    apply_optional_query(
        &mut samples,
        &[
            "uuid",
            "clocks_event_reasons.sw_power_cap",
            "clocks_event_reasons.sw_thermal_slowdown",
            "clocks_event_reasons.hw_thermal_slowdown",
            "clocks_event_reasons.hw_power_brake_slowdown",
            "clocks_event_reasons.hw_slowdown",
            "clocks_event_reasons.sync_boost",
        ],
        "throttle reasons",
        &mut warnings,
        |sample, row| {
            let labels = [
                "software_power_cap",
                "software_thermal_slowdown",
                "hardware_thermal_slowdown",
                "hardware_power_brake",
                "hardware_slowdown",
                "sync_boost",
            ];
            sample.throttle_reasons = labels
                .iter()
                .zip(row.iter().skip(1))
                .filter(|(_, value)| value.trim().eq_ignore_ascii_case("active"))
                .map(|(label, _)| (*label).to_string())
                .collect();
        },
    );
    collect_compute_processes(&mut samples, &mut warnings);

    let mut output = samples.into_values().collect::<Vec<_>>();
    inventory.sort_by(|left, right| {
        (left.gpu_index, left.gpu_uuid.to_ascii_lowercase())
            .cmp(&(right.gpu_index, right.gpu_uuid.to_ascii_lowercase()))
    });
    for (offset, sample) in output.iter_mut().enumerate() {
        sample.sample_sequence = first_sample_sequence
            .checked_add(offset as u64)
            .ok_or_else(|| "GPU telemetry sample sequence overflow".to_string())?;
    }
    Ok(NvidiaTelemetryCollection {
        collector: NVIDIA_SMI_COLLECTOR_VERSION.to_string(),
        samples: output,
        inventory,
        warnings,
    })
}

fn required_query(fields: &[&str]) -> Result<Vec<Vec<String>>, String> {
    run_gpu_query(fields).map_err(|error| format!("NVIDIA telemetry unavailable: {error}"))
}

fn apply_optional_query<F>(
    samples: &mut BTreeMap<String, GpuTelemetrySample>,
    fields: &[&str],
    label: &str,
    warnings: &mut Vec<String>,
    mut apply: F,
) where
    F: FnMut(&mut GpuTelemetrySample, &[String]),
{
    let rows = match run_gpu_query(fields) {
        Ok(rows) => rows,
        Err(error) => {
            warnings.push(format!("{label} metrics unavailable: {error}"));
            return;
        }
    };
    for row in rows {
        let Some(uuid) = row.first().map(|value| value.trim()) else {
            continue;
        };
        if let Some(sample) = samples.get_mut(uuid) {
            apply(sample, &row);
        }
    }
}

fn run_gpu_query(fields: &[&str]) -> Result<Vec<Vec<String>>, String> {
    let output = Command::new("nvidia-smi")
        .arg(format!("--query-gpu={}", fields.join(",")))
        .arg("--format=csv,noheader,nounits")
        .output()
        .map_err(|error| format!("failed to execute nvidia-smi: {error}"))?;
    parse_command_output(output, fields.len())
}

fn collect_compute_processes(
    samples: &mut BTreeMap<String, GpuTelemetrySample>,
    warnings: &mut Vec<String>,
) {
    let output = match Command::new("nvidia-smi")
        .arg("--query-compute-apps=gpu_uuid,pid,process_name,used_gpu_memory")
        .arg("--format=csv,noheader,nounits")
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            warnings.push(format!("compute process metrics unavailable: {error}"));
            return;
        }
    };
    let rows = match parse_command_output(output, 4) {
        Ok(rows) => rows,
        Err(error) => {
            warnings.push(format!("compute process metrics unavailable: {error}"));
            return;
        }
    };
    for row in rows {
        let (Some(uuid), Some(pid), Some(name)) = (row.first(), row.get(1), row.get(2)) else {
            continue;
        };
        let Ok(pid) = pid.trim().parse::<u32>() else {
            continue;
        };
        if let Some(sample) = samples.get_mut(uuid.trim()) {
            sample.processes.push(GpuProcessTelemetry {
                pid,
                process_name: redact_process_name(name),
                used_gpu_memory_mib: parse_optional(row.get(3), parse_u64),
                process_kind: "compute".to_string(),
            });
        }
    }
}

fn parse_command_output(
    output: std::process::Output,
    expected_columns: usize,
) -> Result<Vec<Vec<String>>, String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim().chars().take(240).collect::<String>();
        return Err(if message.is_empty() {
            format!("nvidia-smi exited with {}", output.status)
        } else {
            message
        });
    }
    parse_csv(&output.stdout, expected_columns)
}

fn parse_csv(bytes: &[u8], expected_columns: usize) -> Result<Vec<Vec<String>>, String> {
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .trim(Trim::All)
        .from_reader(bytes);
    reader
        .records()
        .map(|record| {
            let record = record.map_err(|error| format!("invalid nvidia-smi CSV: {error}"))?;
            if record.len() != expected_columns {
                return Err(format!(
                    "nvidia-smi returned {} columns; expected {expected_columns}",
                    record.len()
                ));
            }
            Ok(record.iter().map(ToOwned::to_owned).collect())
        })
        .collect()
}

fn require_columns(row: &[String], expected: usize, label: &str) -> Result<(), String> {
    if row.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} query returned {} columns; expected {expected}",
            row.len()
        ))
    }
}

fn required_value(value: &str, label: &str) -> Result<String, String> {
    optional_text(value).ok_or_else(|| format!("{label} is unavailable"))
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && !is_unavailable(value)).then(|| value.to_string())
}

fn is_unavailable(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "n/a" | "[n/a]" | "not supported" | "[not supported]"
    )
}

fn parse_optional<T>(value: Option<&String>, parser: impl FnOnce(&str) -> Option<T>) -> Option<T> {
    value
        .and_then(|value| optional_text(value))
        .and_then(|value| parser(&value))
}

fn parse_required_u64(value: &str, label: &str) -> Result<u64, String> {
    parse_u64(value).ok_or_else(|| format!("{label} is invalid or unavailable"))
}

fn parse_u64(value: &str) -> Option<u64> {
    value.trim().parse().ok()
}

fn parse_u32(value: &str) -> Option<u32> {
    value.trim().parse().ok()
}

fn parse_f64(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_nonnegative_f64(value: &str) -> Option<f64> {
    parse_f64(value).filter(|value| *value >= 0.0)
}

fn parse_percent(value: &str) -> Option<f64> {
    parse_f64(value).filter(|value| (0.0..=100.0).contains(value))
}

fn parse_pci_device_id(value: &str) -> (Option<String>, Option<String>) {
    let value = value.trim().trim_start_matches("0x");
    if value.len() >= 8 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        (
            Some(value[value.len() - 4..].to_ascii_lowercase()),
            Some(value[..4].to_ascii_lowercase()),
        )
    } else {
        (
            None,
            optional_text(value).map(|value| value.to_ascii_lowercase()),
        )
    }
}

fn redact_process_name(value: &str) -> String {
    value
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .chars()
        .take(128)
        .collect()
}

fn detect_cuda_driver_version() -> Option<String> {
    let output = Command::new("nvidia-smi").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let marker = "CUDA Version:";
    let start = text.find(marker)? + marker.len();
    let version = text[start..]
        .trim_start()
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect::<String>();
    (!version.is_empty()).then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parser_preserves_quoted_process_names() {
        let rows = parse_csv(
            br#"GPU-one,42,"worker, inference.exe",512
"#,
            4,
        )
        .unwrap();
        assert_eq!(rows[0][2], "worker, inference.exe");
    }

    #[test]
    fn process_names_are_redacted_to_basename() {
        assert_eq!(
            redact_process_name(r"C:\Users\provider\private\python.exe"),
            "python.exe"
        );
        assert_eq!(redact_process_name("/opt/jobs/worker"), "worker");
    }

    #[test]
    fn pci_device_id_separates_nvidia_vendor() {
        assert_eq!(
            parse_pci_device_id("0x268410DE"),
            (Some("10de".to_string()), Some("2684".to_string()))
        );
    }

    #[test]
    fn unavailable_values_remain_absent() {
        assert_eq!(parse_optional(Some(&"N/A".to_string()), parse_u64), None);
        assert_eq!(parse_percent("101"), None);
    }
}
