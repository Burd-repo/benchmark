use burd_hardware::collect_nvidia_telemetry;
use burd_protocol::{
    SECURE_RUNTIME_POLICY_VERSION, SECURE_RUNTIME_SCHEMA_VERSION, SecureRuntimeCheck,
    SecureRuntimeImageAllowlistEntry, SecureRuntimePlan, SecureRuntimeResourceLimits,
    SecureRuntimeSecurityProfile, SecureRuntimeTmpfsMount,
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
    pub docker_available: bool,
    pub docker_server_version: Option<String>,
    pub nvidia_runtime_available: bool,
    pub gpu_uuid: Option<String>,
    pub warnings: Vec<String>,
}

pub fn build_secure_runtime_plan(
    agent_version: &str,
    options: SecureRuntimePlanOptions,
) -> SecureRuntimePlan {
    let probe = probe_secure_runtime(&options);
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
    let mut blocking = false;
    let mut verification_required = false;

    let supported_host = probe.host_os == "linux";
    push_check(
        &mut checks,
        "host_os",
        if supported_host { "passed" } else { "failed" },
        if supported_host {
            "Linux host is supported for BN-12 secure runtime."
        } else {
            "BN-12 secure runtime starts on Linux; this host can only build a diagnostic plan."
        },
    );

    if !supported_host {
        warnings.push(format!(
            "secure provider runtime is Linux-first; detected host_os={}",
            probe.host_os
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

    let selected_gpu_uuid =
        normalized_option(options.gpu_uuid.as_deref()).or_else(|| probe.gpu_uuid.clone());
    push_check(
        &mut checks,
        "gpu_uuid_binding",
        if selected_gpu_uuid.is_some() {
            "passed"
        } else {
            "missing"
        },
        selected_gpu_uuid
            .as_ref()
            .map(|gpu_uuid| {
                format!("Runtime plan binds container execution to GPU UUID {gpu_uuid}.")
            })
            .unwrap_or_else(|| {
                "No GPU UUID was supplied or detected; backend lease binding cannot be enforced."
                    .to_string()
            }),
    );
    if selected_gpu_uuid.is_none() {
        verification_required = true;
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

    let status = if !supported_host {
        "unsupported_host"
    } else if blocking {
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
        "BN-12 prepares the secure runtime contract only; customer job execution starts in BN-13."
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
        runtime_engine: "docker+nvidia-container-toolkit".to_string(),
        target_os: probe.host_os,
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

fn probe_secure_runtime(options: &SecureRuntimePlanOptions) -> SecureRuntimeProbe {
    let mut warnings = Vec::new();
    let host_os = std::env::consts::OS.to_string();
    let docker_server_version =
        match command_text("docker", &["version", "--format", "{{.Server.Version}}"]) {
            Ok(version) => Some(version),
            Err(error) => {
                warnings.push(format!("docker version unavailable: {error}"));
                None
            }
        };
    let docker_available = docker_server_version.is_some();
    let nvidia_runtime_available =
        match command_text("docker", &["info", "--format", "{{json .Runtimes}}"]) {
            Ok(output) => output.contains("nvidia"),
            Err(error) => {
                warnings.push(format!("docker NVIDIA runtime probe unavailable: {error}"));
                false
            }
        };
    let gpu_uuid = normalized_option(options.gpu_uuid.as_deref()).or_else(|| {
        match collect_nvidia_telemetry(1) {
            Ok(collection) => collection
                .samples
                .into_iter()
                .next()
                .map(|sample| sample.gpu_uuid),
            Err(error) => {
                warnings.push(format!("GPU UUID auto-detection unavailable: {error}"));
                None
            }
        }
    });

    SecureRuntimeProbe {
        host_os,
        docker_available,
        docker_server_version,
        nvidia_runtime_available,
        gpu_uuid,
        warnings,
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
            docker_available: true,
            docker_server_version: Some("25.0.0".to_string()),
            nvidia_runtime_available: true,
            gpu_uuid: Some("GPU-test".to_string()),
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
                gpu_uuid: None,
                ..ready_probe()
            },
            SecureRuntimePlanOptions::default(),
        );

        assert_eq!(plan.status, "verification_required");
        assert!(plan.docker_args.is_empty());
        assert!(
            plan.checks
                .iter()
                .any(|check| check.id == "gpu_uuid_binding" && check.status == "missing")
        );
    }

    #[test]
    fn non_linux_host_is_diagnostic_only() {
        let plan = calculate_secure_runtime_plan(
            "2026-01-01T00:00:00Z".to_string(),
            "test-agent",
            SecureRuntimeProbe {
                host_os: "windows".to_string(),
                ..ready_probe()
            },
            ready_options(),
        );

        assert_eq!(plan.status, "unsupported_host");
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("Linux-first"))
        );
    }
}
