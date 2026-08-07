use burd_hardware::collect_nvidia_telemetry;
use burd_protocol::{
    PROVIDER_RUNTIME_CAPABILITY_SCHEMA_VERSION, PROVIDER_RUNTIME_VERIFICATION_SCHEMA_VERSION,
    ProviderRuntimeCapability, ProviderRuntimeVerification, SECURE_RUNTIME_POLICY_VERSION,
    SECURE_RUNTIME_SCHEMA_VERSION, SecureRuntimeCheck, SecureRuntimeImageAllowlistEntry,
    SecureRuntimePlan, SecureRuntimeResourceLimits, SecureRuntimeSecurityProfile,
    SecureRuntimeTmpfsMount, validate_provider_runtime_capability,
    validate_provider_runtime_verification,
};
use chrono::Utc;
use std::process::Command;

pub const SECURE_RUNTIME_DEFAULT_TEMPLATE: &str = "llm_inference";
pub const SECURE_RUNTIME_ALLOWED_TEMPLATES: &[&str] = &[
    "llm_inference",
    "embeddings",
    "image_generation",
    "whisper_transcription",
    "file_processing",
];

#[derive(Debug, Clone)]
pub struct SecureRuntimePlanOptions {
    pub template_id: String,
    pub image_ref: Option<String>,
    pub gpu_uuid: Option<String>,
    pub allowed_image_refs: Vec<String>,
    pub cpu_count: Option<f64>,
    pub memory_mib: Option<u64>,
    pub pids_limit: u32,
    pub shm_size_mib: u32,
}

impl Default for SecureRuntimePlanOptions {
    fn default() -> Self {
        Self {
            template_id: SECURE_RUNTIME_DEFAULT_TEMPLATE.to_string(),
            image_ref: None,
            gpu_uuid: None,
            allowed_image_refs: Vec::new(),
            cpu_count: Some(4.0),
            memory_mib: Some(8192),
            pids_limit: 512,
            shm_size_mib: 64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecureRuntimeProbe {
    pub host_os: String,
    pub wsl2_available: Option<bool>,
    pub docker_available: bool,
    pub docker_server_version: Option<String>,
    pub docker_os_type: Option<String>,
    pub docker_operating_system: Option<String>,
    pub docker_kernel_version: Option<String>,
    pub nvidia_runtime_available: bool,
    pub gpu_uuids: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn build_secure_runtime_plan(
    agent_version: &str,
    options: SecureRuntimePlanOptions,
) -> SecureRuntimePlan {
    let probe = probe_secure_runtime();
    calculate_secure_runtime_plan(Utc::now().to_rfc3339(), agent_version, probe, options)
}

pub fn calculate_secure_runtime_plan(
    generated_at: String,
    agent_version: &str,
    probe: SecureRuntimeProbe,
    options: SecureRuntimePlanOptions,
) -> SecureRuntimePlan {
    let mut checks = Vec::new();
    let mut warnings = probe.warnings.clone();
    let capability = calculate_provider_runtime_capability(generated_at.clone(), &probe);
    debug_assert!(validate_provider_runtime_capability(&capability).is_ok());
    let verification = reported_runtime_verification();
    debug_assert!(validate_provider_runtime_verification(&verification).is_ok());
    let mut blocking = capability.status != "ready";
    let mut verification_required = false;

    push_check(
        &mut checks,
        "host_os",
        if matches!(probe.host_os.as_str(), "linux" | "windows") {
            "passed"
        } else {
            "failed"
        },
        match probe.host_os.as_str() {
            "linux" => "Linux host can offer the native Docker backend.".to_string(),
            "windows" => {
                "Windows host is eligible for the WSL2 Linux-container backend.".to_string()
            }
            _ => format!(
                "Host OS {} has no Burd runtime backend definition.",
                probe.host_os
            ),
        },
    );

    push_check(
        &mut checks,
        "runtime_backend",
        if capability.status == "ready" {
            "passed"
        } else if capability.runtime_backend.is_some() {
            "verification_required"
        } else {
            "failed"
        },
        capability
            .runtime_backend
            .as_ref()
            .map(|backend| format!("Detected runtime backend {backend}."))
            .unwrap_or_else(|| "No compatible Linux-container backend was detected.".to_string()),
    );

    if capability.status != "ready" {
        warnings.push(format!(
            "runtime capability is {}; reasons={}",
            capability.status,
            capability.reason_codes.join(",")
        ));
    }

    push_check(
        &mut checks,
        "docker_engine",
        if probe.docker_available {
            "passed"
        } else {
            "failed"
        },
        probe
            .docker_server_version
            .as_ref()
            .map(|version| format!("Docker engine available, server_version={version}."))
            .unwrap_or_else(|| "Docker engine is not reachable from the agent.".to_string()),
    );
    if !probe.docker_available {
        blocking = true;
    }

    push_check(
        &mut checks,
        "nvidia_container_runtime",
        if probe.nvidia_runtime_available {
            "passed"
        } else {
            "failed"
        },
        if probe.nvidia_runtime_available {
            "NVIDIA runtime is advertised by Docker."
        } else {
            "Docker did not advertise the NVIDIA runtime."
        },
    );
    if !probe.nvidia_runtime_available {
        blocking = true;
    }

    let template_allowed = SECURE_RUNTIME_ALLOWED_TEMPLATES
        .iter()
        .any(|template| *template == options.template_id);
    push_check(
        &mut checks,
        "template_allowlist",
        if template_allowed { "passed" } else { "failed" },
        if template_allowed {
            format!(
                "Template {} is approved for BN-12 planning.",
                options.template_id
            )
        } else {
            format!(
                "Template {} is not approved for BN-12 planning.",
                options.template_id
            )
        },
    );
    if !template_allowed {
        blocking = true;
    }

    let selected_image = normalized_option(options.image_ref.as_deref());
    let image_has_digest = selected_image
        .as_ref()
        .is_some_and(|image_ref| image_ref.contains("@sha256:"));
    let image_allowlisted = selected_image.as_ref().is_some_and(|image_ref| {
        options
            .allowed_image_refs
            .iter()
            .any(|allowed| allowed == image_ref)
    });
    let image_status = match (&selected_image, image_has_digest, image_allowlisted) {
        (None, _, _) => "missing",
        (Some(_), false, _) => "failed",
        (Some(_), true, true) => "passed",
        (Some(_), true, false) => "failed",
    };
    let image_summary = match (&selected_image, image_has_digest, image_allowlisted) {
        (None, _, _) => {
            "No image reference was supplied; job execution remains unavailable.".to_string()
        }
        (Some(_), false, _) => "Image reference must be pinned with @sha256 digest.".to_string(),
        (Some(image_ref), true, true) => {
            format!("Image {image_ref} is digest-pinned and allowlisted.")
        }
        (Some(image_ref), true, false) => {
            format!("Image {image_ref} is not present in the allowlist.")
        }
    };
    push_check(
        &mut checks,
        "signed_image_allowlist",
        image_status,
        image_summary,
    );
    match (&selected_image, image_has_digest, image_allowlisted) {
        (None, _, _) => verification_required = true,
        (Some(_), true, true) => {}
        _ => blocking = true,
    }

    let requested_gpu_uuid = normalized_option(options.gpu_uuid.as_deref());
    let selected_gpu_uuid = requested_gpu_uuid
        .clone()
        .or_else(|| probe.gpu_uuids.first().cloned());
    let selected_gpu_observed = selected_gpu_uuid
        .as_ref()
        .is_some_and(|gpu_uuid| probe.gpu_uuids.contains(gpu_uuid));
    push_check(
        &mut checks,
        "gpu_uuid_binding",
        if selected_gpu_observed {
            "passed"
        } else if selected_gpu_uuid.is_some() {
            "failed"
        } else {
            "missing"
        },
        match (&selected_gpu_uuid, selected_gpu_observed) {
            (Some(gpu_uuid), true) => {
                format!("Runtime plan binds container execution to GPU UUID {gpu_uuid}.")
            }
            (Some(_), false) => {
                "Requested GPU UUID was not observed in the local runtime capability.".to_string()
            }
            (None, _) => {
                "No GPU UUID was supplied or detected; backend lease binding cannot be enforced."
                    .to_string()
            }
        },
    );
    if selected_gpu_uuid.is_none() {
        verification_required = true;
    } else if !selected_gpu_observed {
        blocking = true;
    }

    let resources_valid = resources_are_valid(&options);
    push_check(
        &mut checks,
        "resource_limits",
        if resources_valid { "passed" } else { "failed" },
        if resources_valid {
            "CPU, memory, PID, and shared-memory limits are explicit."
        } else {
            "CPU, memory, PID, and shared-memory limits must be finite positive values."
        },
    );
    if !resources_valid {
        blocking = true;
    }

    push_check(
        &mut checks,
        "security_profile",
        "passed",
        "Root filesystem is read-only, non-root user is enforced, capabilities are dropped, no-new-privileges is enabled, seccomp is required, network is disabled, and tmpfs mounts are explicit.",
    );

    let resources = SecureRuntimeResourceLimits {
        cpu_count: options.cpu_count,
        memory_mib: options.memory_mib,
        pids_limit: options.pids_limit,
        shm_size_mib: options.shm_size_mib,
    };
    let security = default_security_profile();
    let image_allowlist = options
        .allowed_image_refs
        .iter()
        .filter_map(|image_ref| normalized_option(Some(image_ref.as_str())))
        .map(|image_ref| SecureRuntimeImageAllowlistEntry {
            signature_status: if image_ref.contains("@sha256:") {
                "digest_allowlisted".to_string()
            } else {
                "unusable_without_digest".to_string()
            },
            signer: None,
            image_ref,
        })
        .collect::<Vec<_>>();

    let status = if blocking {
        "blocked"
    } else if verification_required {
        "verification_required"
    } else {
        "ready"
    }
    .to_string();

    let docker_args = if status == "ready" {
        match (&selected_image, &selected_gpu_uuid) {
            (Some(image_ref), Some(gpu_uuid)) => build_docker_args(image_ref, gpu_uuid, &resources),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let mut notes = vec![
        format!("agent_version={agent_version}"),
        "The runtime capability is agent-reported and is not scheduler-authoritative.".to_string(),
        "Control Plane runtime proof is required before a reported capability can become verified."
            .to_string(),
        "The agent must not run arbitrary shell payloads from customers.".to_string(),
        "Runtime execution must be bound to a backend lease before paid jobs are accepted."
            .to_string(),
    ];
    if selected_image.is_some() && !image_allowlisted {
        notes.push("Image signature verification is represented by digest allowlisting in this local slice; backend/admin signing policy will become authoritative for jobs.".to_string());
    }

    SecureRuntimePlan {
        schema_version: SECURE_RUNTIME_SCHEMA_VERSION.to_string(),
        policy_version: SECURE_RUNTIME_POLICY_VERSION.to_string(),
        generated_at,
        status,
        capability,
        verification,
        template_id: options.template_id,
        image_ref: selected_image,
        gpu_uuid: selected_gpu_uuid,
        image_allowlist,
        resources,
        security,
        docker_args,
        checks,
        warnings,
        notes,
    }
}

pub fn calculate_provider_runtime_capability(
    observed_at: String,
    probe: &SecureRuntimeProbe,
) -> ProviderRuntimeCapability {
    let supported_host = matches!(probe.host_os.as_str(), "linux" | "windows");
    let docker_linux_engine = probe.docker_available
        && probe
            .docker_os_type
            .as_deref()
            .is_some_and(|os_type| os_type.eq_ignore_ascii_case("linux"));
    let wsl2_engine = probe.wsl2_available == Some(true)
        && docker_linux_engine
        && probe.host_os == "windows"
        && probe
            .docker_kernel_version
            .as_deref()
            .is_some_and(|version| {
                let version = version.to_ascii_lowercase();
                version.contains("microsoft-standard-wsl2") || version.contains("wsl2")
            });

    let runtime_backend = match probe.host_os.as_str() {
        "linux" if docker_linux_engine => Some("docker_linux_native".to_string()),
        "windows" if wsl2_engine => Some("docker_wsl2".to_string()),
        _ => None,
    };
    let runtime_provider = if probe.docker_available {
        Some(
            if probe
                .docker_operating_system
                .as_deref()
                .is_some_and(|name| name.to_ascii_lowercase().contains("docker desktop"))
            {
                "docker_desktop"
            } else {
                "docker_engine"
            }
            .to_string(),
        )
    } else {
        None
    };

    let mut reason_codes = Vec::new();
    if !supported_host {
        push_reason(&mut reason_codes, "unsupported_host_os");
    } else {
        if probe.host_os == "windows" && probe.wsl2_available != Some(true) {
            push_reason(&mut reason_codes, "wsl2_unavailable");
        }
        if !probe.docker_available {
            push_reason(&mut reason_codes, "docker_unavailable");
        } else if !docker_linux_engine {
            push_reason(&mut reason_codes, "linux_container_engine_unavailable");
        }
        if probe.host_os == "windows" && probe.docker_available && !wsl2_engine {
            push_reason(&mut reason_codes, "wsl2_runtime_unavailable");
        }
        if probe.docker_available && !probe.nvidia_runtime_available {
            push_reason(&mut reason_codes, "nvidia_runtime_unavailable");
        }
        if probe.gpu_uuids.is_empty() {
            push_reason(&mut reason_codes, "gpu_uuid_unavailable");
        }
        if probe.host_os == "windows" && runtime_backend.as_deref() == Some("docker_wsl2") {
            push_reason(&mut reason_codes, "runtime_backend_verification_required");
        }
    }

    let status = if !supported_host {
        "unsupported"
    } else if reason_codes.is_empty() {
        "ready"
    } else {
        "not_ready"
    };

    ProviderRuntimeCapability {
        schema_version: PROVIDER_RUNTIME_CAPABILITY_SCHEMA_VERSION.to_string(),
        observed_at,
        host_os: probe.host_os.clone(),
        runtime_backend,
        runtime_provider,
        container_os: "linux".to_string(),
        gpu_backend: "cuda".to_string(),
        gpu_runtime: "nvidia".to_string(),
        isolation_mode: "linux_container".to_string(),
        status: status.to_string(),
        reason_codes,
        gpu_uuids: probe.gpu_uuids.clone(),
    }
}

fn reported_runtime_verification() -> ProviderRuntimeVerification {
    ProviderRuntimeVerification {
        schema_version: PROVIDER_RUNTIME_VERIFICATION_SCHEMA_VERSION.to_string(),
        authority: "agent".to_string(),
        status: "reported".to_string(),
        gpu_uuid_binding: "unverified".to_string(),
        reason_codes: vec!["runtime_proof_required".to_string()],
    }
}

fn push_reason(reason_codes: &mut Vec<String>, reason: &str) {
    if !reason_codes.iter().any(|existing| existing == reason) {
        reason_codes.push(reason.to_string());
    }
}

fn probe_secure_runtime() -> SecureRuntimeProbe {
    let mut warnings = Vec::new();
    let host_os = std::env::consts::OS.to_string();
    let wsl2_available = if host_os == "windows" {
        match command_text("wsl.exe", &["--status"]) {
            Ok(_) => Some(true),
            Err(error) => {
                warnings.push(format!("WSL2 status probe unavailable: {error}"));
                Some(false)
            }
        }
    } else {
        None
    };
    let docker_server_version =
        match command_text("docker", &["version", "--format", "{{.Server.Version}}"]) {
            Ok(version) => Some(version),
            Err(error) => {
                warnings.push(format!("docker version unavailable: {error}"));
                None
            }
        };
    let docker_available = docker_server_version.is_some();
    let (docker_os_type, docker_operating_system, docker_kernel_version) = if docker_available {
        (
            docker_probe_field("{{.OSType}}", "OS type", &mut warnings),
            docker_probe_field("{{.OperatingSystem}}", "operating system", &mut warnings),
            docker_probe_field("{{.KernelVersion}}", "kernel version", &mut warnings),
        )
    } else {
        (None, None, None)
    };
    let nvidia_runtime_available = if docker_available {
        match command_text("docker", &["info", "--format", "{{json .Runtimes}}"]) {
            Ok(output) => output.contains("nvidia"),
            Err(error) => {
                warnings.push(format!("docker NVIDIA runtime probe unavailable: {error}"));
                false
            }
        }
    } else {
        false
    };
    let gpu_uuids = match collect_nvidia_telemetry(1) {
        Ok(collection) => collection
            .samples
            .into_iter()
            .map(|sample| sample.gpu_uuid)
            .collect(),
        Err(error) => {
            warnings.push(format!("GPU UUID auto-detection unavailable: {error}"));
            Vec::new()
        }
    };

    SecureRuntimeProbe {
        host_os,
        wsl2_available,
        docker_available,
        docker_server_version,
        docker_os_type,
        docker_operating_system,
        docker_kernel_version,
        nvidia_runtime_available,
        gpu_uuids,
        warnings,
    }
}

fn docker_probe_field(format: &str, label: &str, warnings: &mut Vec<String>) -> Option<String> {
    match command_text("docker", &["info", "--format", format]) {
        Ok(value) => normalized_option(Some(&value)),
        Err(error) => {
            warnings.push(format!("docker {label} probe unavailable: {error}"));
            None
        }
    }
}

fn command_text(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("failed to execute {program}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim().chars().take(240).collect::<String>();
        return Err(if message.is_empty() {
            format!("{program} exited with {}", output.status)
        } else {
            message
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn default_security_profile() -> SecureRuntimeSecurityProfile {
    SecureRuntimeSecurityProfile {
        read_only_rootfs: true,
        run_as_user: "1000:1000".to_string(),
        no_new_privileges: true,
        cap_drop: vec!["ALL".to_string()],
        seccomp_profile: "default".to_string(),
        ipc_mode: "none".to_string(),
        network_mode: "none".to_string(),
        tmpfs_mounts: vec![
            SecureRuntimeTmpfsMount {
                target: "/tmp".to_string(),
                size_mib: 1024,
                options: vec![
                    "rw".to_string(),
                    "noexec".to_string(),
                    "nosuid".to_string(),
                    "nodev".to_string(),
                ],
            },
            SecureRuntimeTmpfsMount {
                target: "/run/burd-secrets".to_string(),
                size_mib: 16,
                options: vec![
                    "rw".to_string(),
                    "noexec".to_string(),
                    "nosuid".to_string(),
                    "nodev".to_string(),
                    "mode=0700".to_string(),
                ],
            },
        ],
        secrets_mode: "ephemeral_tmpfs".to_string(),
        cleanup_required: true,
        arbitrary_shell_allowed: false,
    }
}

fn build_docker_args(
    image_ref: &str,
    gpu_uuid: &str,
    resources: &SecureRuntimeResourceLimits,
) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--pull".to_string(),
        "never".to_string(),
        "--name".to_string(),
        "burd-runtime-lease-placeholder".to_string(),
        "--gpus".to_string(),
        format!("device={gpu_uuid}"),
        "--read-only".to_string(),
        "--user".to_string(),
        "1000:1000".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
        "--security-opt".to_string(),
        "no-new-privileges".to_string(),
        "--security-opt".to_string(),
        "seccomp=default".to_string(),
        "--pids-limit".to_string(),
        resources.pids_limit.to_string(),
        "--network".to_string(),
        "none".to_string(),
        "--ipc".to_string(),
        "none".to_string(),
        "--shm-size".to_string(),
        format!("{}m", resources.shm_size_mib),
        "--tmpfs".to_string(),
        "/tmp:rw,noexec,nosuid,nodev,size=1024m".to_string(),
        "--tmpfs".to_string(),
        "/run/burd-secrets:rw,noexec,nosuid,nodev,size=16m,mode=0700".to_string(),
    ];
    if let Some(memory_mib) = resources.memory_mib {
        args.push("--memory".to_string());
        args.push(format!("{memory_mib}m"));
    }
    if let Some(cpu_count) = resources.cpu_count {
        args.push("--cpus".to_string());
        args.push(format_cpu_count(cpu_count));
    }
    args.push(image_ref.to_string());
    args
}

fn resources_are_valid(options: &SecureRuntimePlanOptions) -> bool {
    let cpu_valid = options
        .cpu_count
        .is_none_or(|value| value.is_finite() && value > 0.0);
    let memory_valid = options.memory_mib.is_none_or(|value| value > 0);
    cpu_valid && memory_valid && options.pids_limit > 0 && options.shm_size_mib > 0
}

fn format_cpu_count(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn normalized_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn push_check(
    checks: &mut Vec<SecureRuntimeCheck>,
    id: &str,
    status: &str,
    summary: impl Into<String>,
) {
    checks.push(SecureRuntimeCheck {
        id: id.to_string(),
        status: status.to_string(),
        summary: summary.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_probe() -> SecureRuntimeProbe {
        SecureRuntimeProbe {
            host_os: "linux".to_string(),
            wsl2_available: None,
            docker_available: true,
            docker_server_version: Some("25.0.0".to_string()),
            docker_os_type: Some("linux".to_string()),
            docker_operating_system: Some("Ubuntu 24.04".to_string()),
            docker_kernel_version: Some("6.8.0".to_string()),
            nvidia_runtime_available: true,
            gpu_uuids: vec!["GPU-test".to_string()],
            warnings: Vec::new(),
        }
    }

    fn ready_options() -> SecureRuntimePlanOptions {
        SecureRuntimePlanOptions {
            image_ref: Some("ghcr.io/burd/runtime/llm@sha256:abcdef".to_string()),
            allowed_image_refs: vec!["ghcr.io/burd/runtime/llm@sha256:abcdef".to_string()],
            ..SecureRuntimePlanOptions::default()
        }
    }

    #[test]
    fn ready_linux_plan_contains_hardened_docker_args() {
        let plan = calculate_secure_runtime_plan(
            "2026-01-01T00:00:00Z".to_string(),
            "test-agent",
            ready_probe(),
            ready_options(),
        );

        assert_eq!(plan.status, "ready");
        assert_eq!(plan.capability.host_os, "linux");
        assert_eq!(
            plan.capability.runtime_backend.as_deref(),
            Some("docker_linux_native")
        );
        assert_eq!(plan.capability.container_os, "linux");
        assert_eq!(plan.verification.status, "reported");
        assert_eq!(plan.verification.gpu_uuid_binding, "unverified");
        assert_eq!(plan.gpu_uuid.as_deref(), Some("GPU-test"));
        assert!(plan.docker_args.iter().any(|arg| arg == "--read-only"));
        assert!(plan.docker_args.iter().any(|arg| arg == "--cap-drop"));
        assert!(plan.docker_args.iter().any(|arg| arg == "--gpus"));
        assert!(plan.docker_args.iter().any(|arg| arg == "--network"));
        assert!(!plan.security.arbitrary_shell_allowed);
    }

    #[test]
    fn digest_image_must_be_allowlisted() {
        let mut options = ready_options();
        options.allowed_image_refs.clear();
        let plan = calculate_secure_runtime_plan(
            "2026-01-01T00:00:00Z".to_string(),
            "test-agent",
            ready_probe(),
            options,
        );

        assert_eq!(plan.status, "blocked");
        assert!(plan.docker_args.is_empty());
        assert!(
            plan.checks
                .iter()
                .any(|check| check.id == "signed_image_allowlist" && check.status == "failed")
        );
    }

    #[test]
    fn missing_image_and_gpu_requires_backend_verification() {
        let plan = calculate_secure_runtime_plan(
            "2026-01-01T00:00:00Z".to_string(),
            "test-agent",
            SecureRuntimeProbe {
                gpu_uuids: Vec::new(),
                ..ready_probe()
            },
            SecureRuntimePlanOptions::default(),
        );

        assert_eq!(plan.status, "blocked");
        assert_eq!(plan.capability.status, "not_ready");
        assert!(
            plan.capability
                .reason_codes
                .iter()
                .any(|reason| reason == "gpu_uuid_unavailable")
        );
        assert!(plan.docker_args.is_empty());
        assert!(
            plan.checks
                .iter()
                .any(|check| check.id == "gpu_uuid_binding" && check.status == "missing")
        );
    }

    #[test]
    fn windows_wsl2_is_not_globally_unsupported() {
        let plan = calculate_secure_runtime_plan(
            "2026-01-01T00:00:00Z".to_string(),
            "test-agent",
            SecureRuntimeProbe {
                host_os: "windows".to_string(),
                wsl2_available: Some(true),
                docker_operating_system: Some("Docker Desktop".to_string()),
                docker_kernel_version: Some("5.15.167.4-microsoft-standard-WSL2".to_string()),
                ..ready_probe()
            },
            ready_options(),
        );

        assert_eq!(plan.status, "blocked");
        assert_eq!(plan.capability.status, "not_ready");
        assert_eq!(
            plan.capability.runtime_backend.as_deref(),
            Some("docker_wsl2")
        );
        assert_eq!(
            plan.capability.runtime_provider.as_deref(),
            Some("docker_desktop")
        );
        assert!(
            plan.capability
                .reason_codes
                .iter()
                .any(|reason| reason == "runtime_backend_verification_required")
        );
        assert!(!serde_json::to_string(&plan).unwrap().contains("target_os"));
        assert!(plan.docker_args.is_empty());
    }

    #[test]
    fn windows_without_docker_is_not_ready_with_actionable_reason() {
        let capability = calculate_provider_runtime_capability(
            "2026-01-01T00:00:00Z".to_string(),
            &SecureRuntimeProbe {
                host_os: "windows".to_string(),
                wsl2_available: Some(false),
                docker_available: false,
                docker_server_version: None,
                docker_os_type: None,
                docker_operating_system: None,
                docker_kernel_version: None,
                nvidia_runtime_available: false,
                gpu_uuids: vec!["GPU-test".to_string()],
                warnings: Vec::new(),
            },
        );

        assert_eq!(capability.status, "not_ready");
        assert_eq!(
            capability.reason_codes,
            ["wsl2_unavailable", "docker_unavailable"]
        );
        assert!(capability.runtime_backend.is_none());
    }

    #[test]
    fn undefined_host_backend_is_reported_as_unsupported() {
        let capability = calculate_provider_runtime_capability(
            "2026-01-01T00:00:00Z".to_string(),
            &SecureRuntimeProbe {
                host_os: "macos".to_string(),
                ..ready_probe()
            },
        );

        assert_eq!(capability.status, "unsupported");
        assert!(capability.runtime_backend.is_none());
        assert_eq!(capability.reason_codes, ["unsupported_host_os"]);
    }
}
