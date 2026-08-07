use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;

pub const MAX_DOCKER_LOG_BYTES: usize = 64 * 1024;
pub const MAX_DOCKER_LOG_LINES: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerContainerPlan {
    pub name: String,
    pub image_ref: String,
    pub gpu_uuid: String,
    pub user: String,
    pub cpu_millis: u32,
    pub memory_mib: u64,
    pub pids_limit: u32,
    pub shm_size_mib: u64,
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExistingDockerContainer {
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerContainerState {
    pub running: bool,
    pub exit_code: Option<i32>,
    pub oom_killed: bool,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DockerContainerLogs {
    stdout_tail: String,
    stderr_tail: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

impl DockerContainerLogs {
    pub fn new(stdout: &str, stderr: &str, stdout_truncated: bool, stderr_truncated: bool) -> Self {
        let (stdout, stdout_bounded) = bounded_text_tail(stdout, MAX_DOCKER_LOG_BYTES);
        let (stderr, stderr_bounded) = bounded_text_tail(stderr, MAX_DOCKER_LOG_BYTES);
        let stdout_truncated = stdout_truncated || stdout_bounded;
        let stderr_truncated = stderr_truncated || stderr_bounded;
        let stdout = discard_partial_leading_line(stdout, stdout_truncated);
        let stderr = discard_partial_leading_line(stderr, stderr_truncated);
        Self {
            stdout_tail: redact_log_text(&stdout),
            stderr_tail: redact_log_text(&stderr),
            stdout_truncated,
            stderr_truncated,
        }
    }

    pub fn stdout_tail(&self) -> &str {
        &self.stdout_tail
    }

    pub fn stderr_tail(&self) -> &str {
        &self.stderr_tail
    }

    pub fn stdout_truncated(&self) -> bool {
        self.stdout_truncated
    }

    pub fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DockerRuntimeError {
    code: &'static str,
}

impl DockerRuntimeError {
    pub const fn new(code: &'static str) -> Self {
        Self { code }
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Debug for DockerRuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerRuntimeError")
            .field("code", &self.code)
            .finish()
    }
}

impl Display for DockerRuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "Docker runtime operation failed ({})", self.code)
    }
}

impl std::error::Error for DockerRuntimeError {}

pub trait DockerRuntimeBackend: Send + Sync + 'static {
    fn runtime_backend(&self) -> &'static str;

    /// Performs read-only host, Docker, image and GPU checks before container creation.
    fn verify_environment(&self, plan: &DockerContainerPlan) -> Result<(), DockerRuntimeError>;

    fn existing_container(
        &self,
        name: &str,
    ) -> Result<Option<ExistingDockerContainer>, DockerRuntimeError>;

    fn create(&self, plan: &DockerContainerPlan) -> Result<String, DockerRuntimeError>;
    fn start(&self, container_id: &str) -> Result<(), DockerRuntimeError>;
    fn inspect(&self, container_id: &str) -> Result<DockerContainerState, DockerRuntimeError>;

    /// Returns already bounded and redacted tails; implementations must never expose raw logs.
    fn logs(&self, container_id: &str) -> Result<DockerContainerLogs, DockerRuntimeError>;
    fn stop(&self, container_id: &str, grace_seconds: u32) -> Result<(), DockerRuntimeError>;
    fn kill(&self, container_id: &str) -> Result<(), DockerRuntimeError>;
    fn remove(&self, container_id_or_name: &str) -> Result<(), DockerRuntimeError>;
}

#[derive(Clone, Debug)]
pub struct LinuxNativeDockerBackend {
    docker_program: String,
    nvidia_smi_program: String,
}

impl Default for LinuxNativeDockerBackend {
    fn default() -> Self {
        Self {
            docker_program: "docker".to_string(),
            nvidia_smi_program: "nvidia-smi".to_string(),
        }
    }
}

impl LinuxNativeDockerBackend {
    fn docker(&self, args: &[String]) -> Result<BoundedCommandOutput, DockerRuntimeError> {
        run_bounded_command(&self.docker_program, args)
    }

    fn successful_docker(
        &self,
        args: &[String],
        code: &'static str,
    ) -> Result<BoundedCommandOutput, DockerRuntimeError> {
        let output = self.docker(args)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(DockerRuntimeError::new(code))
        }
    }

    pub fn create_args(plan: &DockerContainerPlan) -> Vec<String> {
        let mut args = vec![
            "create".to_string(),
            "--pull".to_string(),
            "never".to_string(),
            "--name".to_string(),
            plan.name.clone(),
            "--restart".to_string(),
            "no".to_string(),
            "--no-healthcheck".to_string(),
        ];
        for (key, value) in &plan.labels {
            args.push("--label".to_string());
            args.push(format!("{key}={value}"));
        }
        args.extend([
            "--gpus".to_string(),
            format!("device={}", plan.gpu_uuid),
            "--read-only".to_string(),
            "--user".to_string(),
            plan.user.clone(),
            "--cap-drop".to_string(),
            "ALL".to_string(),
            "--security-opt".to_string(),
            "no-new-privileges".to_string(),
            "--security-opt".to_string(),
            // Runtime policy calls this profile `default`; Docker CLI names its
            // built-in default profile `builtin`.
            "seccomp=builtin".to_string(),
            "--network".to_string(),
            "none".to_string(),
            "--ipc".to_string(),
            "none".to_string(),
            "--pids-limit".to_string(),
            plan.pids_limit.to_string(),
            "--memory".to_string(),
            format!("{}m", plan.memory_mib),
            "--memory-swap".to_string(),
            format!("{}m", plan.memory_mib),
            "--cpus".to_string(),
            format_cpu_millis(plan.cpu_millis),
            "--shm-size".to_string(),
            format!("{}m", plan.shm_size_mib),
            "--tmpfs".to_string(),
            "/tmp:rw,noexec,nosuid,nodev,size=1024m".to_string(),
            "--tmpfs".to_string(),
            "/run/burd-secrets:rw,noexec,nosuid,nodev,size=16m,mode=0700".to_string(),
            plan.image_ref.clone(),
        ]);
        args
    }
}

impl DockerRuntimeBackend for LinuxNativeDockerBackend {
    fn runtime_backend(&self) -> &'static str {
        "docker_linux_native"
    }

    fn verify_environment(&self, plan: &DockerContainerPlan) -> Result<(), DockerRuntimeError> {
        if std::env::consts::OS != "linux" {
            return Err(DockerRuntimeError::new("linux_native_host_required"));
        }

        let server_os = self.successful_docker(
            &["version".into(), "--format".into(), "{{.Server.Os}}".into()],
            "docker_unavailable",
        )?;
        if server_os.stdout.trim() != "linux" {
            return Err(DockerRuntimeError::new("linux_container_engine_required"));
        }

        let runtimes = self.successful_docker(
            &[
                "info".into(),
                "--format".into(),
                "{{json .Runtimes}}".into(),
            ],
            "docker_runtime_probe_failed",
        )?;
        let runtime_map: serde_json::Value = serde_json::from_str(runtimes.stdout.trim())
            .map_err(|_| DockerRuntimeError::new("docker_runtime_probe_invalid"))?;
        if runtime_map.get("nvidia").is_none() {
            return Err(DockerRuntimeError::new("nvidia_runtime_unavailable"));
        }

        let gpu_output = run_bounded_command(
            &self.nvidia_smi_program,
            &["--query-gpu=uuid".into(), "--format=csv,noheader".into()],
        )?;
        if !gpu_output.status.success()
            || !gpu_output
                .stdout
                .lines()
                .map(str::trim)
                .any(|uuid| uuid.eq_ignore_ascii_case(&plan.gpu_uuid))
        {
            return Err(DockerRuntimeError::new("gpu_uuid_unavailable"));
        }

        let image = self.successful_docker(
            &[
                "image".into(),
                "inspect".into(),
                "--format".into(),
                "{{.Id}}".into(),
                plan.image_ref.clone(),
            ],
            "container_image_unavailable",
        )?;
        if image.stdout.trim().is_empty() {
            return Err(DockerRuntimeError::new("container_image_invalid"));
        }
        Ok(())
    }

    fn existing_container(
        &self,
        name: &str,
    ) -> Result<Option<ExistingDockerContainer>, DockerRuntimeError> {
        let output = self.docker(&[
            "container".into(),
            "inspect".into(),
            "--format".into(),
            "{{json .Config.Labels}}".into(),
            name.to_string(),
        ])?;
        if !output.status.success() {
            return Ok(None);
        }
        let labels = serde_json::from_str::<Option<BTreeMap<String, String>>>(output.stdout.trim())
            .map_err(|_| DockerRuntimeError::new("container_labels_invalid"))?
            .unwrap_or_default();
        Ok(Some(ExistingDockerContainer { labels }))
    }

    fn create(&self, plan: &DockerContainerPlan) -> Result<String, DockerRuntimeError> {
        let output = self.successful_docker(&Self::create_args(plan), "container_create_failed")?;
        let id = output.stdout.trim();
        if !(12..=64).contains(&id.len()) || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DockerRuntimeError::new("container_id_invalid"));
        }
        Ok(id.to_string())
    }

    fn start(&self, container_id: &str) -> Result<(), DockerRuntimeError> {
        self.successful_docker(
            &["start".into(), container_id.to_string()],
            "container_start_failed",
        )?;
        Ok(())
    }

    fn inspect(&self, container_id: &str) -> Result<DockerContainerState, DockerRuntimeError> {
        let output = self.successful_docker(
            &[
                "container".into(),
                "inspect".into(),
                "--format".into(),
                "{{json .State}}".into(),
                container_id.to_string(),
            ],
            "container_inspect_failed",
        )?;
        let state: DockerStateOutput = serde_json::from_str(output.stdout.trim())
            .map_err(|_| DockerRuntimeError::new("container_state_invalid"))?;
        Ok(DockerContainerState {
            running: state.running,
            exit_code: (!state.running).then_some(state.exit_code),
            oom_killed: state.oom_killed,
            started_at: normalized_docker_timestamp(state.started_at),
            finished_at: normalized_docker_timestamp(state.finished_at),
        })
    }

    fn logs(&self, container_id: &str) -> Result<DockerContainerLogs, DockerRuntimeError> {
        let output = self.successful_docker(
            &[
                "logs".into(),
                "--tail".into(),
                MAX_DOCKER_LOG_LINES.to_string(),
                container_id.to_string(),
            ],
            "container_logs_failed",
        )?;
        Ok(DockerContainerLogs::new(
            &output.stdout,
            &output.stderr,
            output.stdout_truncated,
            output.stderr_truncated,
        ))
    }

    fn stop(&self, container_id: &str, grace_seconds: u32) -> Result<(), DockerRuntimeError> {
        self.successful_docker(
            &[
                "stop".into(),
                "--time".into(),
                grace_seconds.to_string(),
                container_id.to_string(),
            ],
            "container_stop_failed",
        )?;
        Ok(())
    }

    fn kill(&self, container_id: &str) -> Result<(), DockerRuntimeError> {
        self.successful_docker(
            &["kill".into(), container_id.to_string()],
            "container_kill_failed",
        )?;
        Ok(())
    }

    fn remove(&self, container_id_or_name: &str) -> Result<(), DockerRuntimeError> {
        self.successful_docker(
            &[
                "rm".into(),
                "--force".into(),
                container_id_or_name.to_string(),
            ],
            "container_remove_failed",
        )?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerStateOutput {
    running: bool,
    exit_code: i32,
    #[serde(rename = "OOMKilled")]
    oom_killed: bool,
    started_at: String,
    finished_at: String,
}

fn normalized_docker_timestamp(value: String) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with("0001-01-01") {
        None
    } else {
        Some(value.to_string())
    }
}

fn format_cpu_millis(cpu_millis: u32) -> String {
    let whole = cpu_millis / 1_000;
    let fraction = cpu_millis % 1_000;
    if fraction == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{fraction:03}")
            .trim_end_matches('0')
            .to_string()
    }
}

struct BoundedCommandOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

fn run_bounded_command(
    program: &str,
    args: &[String],
) -> Result<BoundedCommandOutput, DockerRuntimeError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| DockerRuntimeError::new("runtime_command_spawn_failed"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| DockerRuntimeError::new("runtime_stdout_unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| DockerRuntimeError::new("runtime_stderr_unavailable"))?;
    let stdout_reader = thread::spawn(move || read_tail_bounded(stdout, MAX_DOCKER_LOG_BYTES));
    let stderr_reader = thread::spawn(move || read_tail_bounded(stderr, MAX_DOCKER_LOG_BYTES));
    let status = child
        .wait()
        .map_err(|_| DockerRuntimeError::new("runtime_command_wait_failed"))?;
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| DockerRuntimeError::new("runtime_stdout_reader_failed"))?
        .map_err(|_| DockerRuntimeError::new("runtime_stdout_read_failed"))?;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| DockerRuntimeError::new("runtime_stderr_reader_failed"))?
        .map_err(|_| DockerRuntimeError::new("runtime_stderr_read_failed"))?;
    Ok(BoundedCommandOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        stdout_truncated,
        stderr_truncated,
    })
}

fn read_tail_bounded<R: Read>(mut reader: R, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut kept = Vec::with_capacity(limit);
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if count >= limit {
            kept.clear();
            kept.extend_from_slice(&buffer[count - limit..count]);
            truncated = true;
            continue;
        }
        let overflow = kept.len().saturating_add(count).saturating_sub(limit);
        if overflow > 0 {
            kept.drain(..overflow);
            truncated = true;
        }
        kept.extend_from_slice(&buffer[..count]);
    }
    Ok((kept, truncated))
}

fn bounded_text_tail(value: &str, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        return (value.to_string(), false);
    }
    let mut start = value.len() - limit;
    while !value.is_char_boundary(start) {
        start += 1;
    }
    (value[start..].to_string(), true)
}

fn discard_partial_leading_line(value: String, truncated: bool) -> String {
    if !truncated {
        return value;
    }
    value
        .find('\n')
        .map(|newline| value[newline + 1..].to_string())
        .unwrap_or_default()
}

pub fn redact_log_text(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "authorization",
                "bearer",
                "credential",
                "jobcred",
                "secret",
                "token",
                "private_key",
            ]
            .iter()
            .any(|marker| lower.contains(marker))
            {
                "[REDACTED]".to_string()
            } else {
                line.chars()
                    .filter(|character| !character.is_control() || *character == '\t')
                    .collect()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> DockerContainerPlan {
        DockerContainerPlan {
            name: "burd-job-job_1-deadbeef".to_string(),
            image_ref: format!("ghcr.io/burd/runtime/llm@sha256:{}", "a".repeat(64)),
            gpu_uuid: "GPU-test".to_string(),
            user: "1000:1000".to_string(),
            cpu_millis: 4_250,
            memory_mib: 8_192,
            pids_limit: 512,
            shm_size_mib: 64,
            labels: BTreeMap::from([
                ("com.burd.managed".to_string(), "true".to_string()),
                ("com.burd.job_id".to_string(), "job_1".to_string()),
            ]),
        }
    }

    #[test]
    fn create_args_are_structured_and_hardened() {
        let plan = plan();
        let args = LinuxNativeDockerBackend::create_args(&plan);
        for required in [
            "create",
            "--pull",
            "never",
            "--read-only",
            "--restart",
            "--no-healthcheck",
            "--cap-drop",
            "ALL",
            "no-new-privileges",
            "seccomp=builtin",
            "--network",
            "none",
            "--ipc",
            "--pids-limit",
            "--memory",
            "--memory-swap",
            "--cpus",
            "--shm-size",
            "--tmpfs",
        ] {
            assert!(args.iter().any(|arg| arg == required), "missing {required}");
        }
        assert!(args.iter().any(|arg| arg == "device=GPU-test"));
        assert!(args.iter().any(|arg| arg == "4.25"));
        assert_eq!(args.last(), Some(&plan.image_ref));
    }

    #[test]
    fn create_args_never_enable_host_or_privileged_access() {
        let joined = LinuxNativeDockerBackend::create_args(&plan()).join(" ");
        for forbidden in [
            "--privileged",
            "--pid=host",
            "--network=host",
            "--ipc=host",
            "/var/run/docker.sock",
            "--entrypoint",
            "-v ",
            "--volume",
            "sh -c",
        ] {
            assert!(!joined.contains(forbidden), "found {forbidden}");
        }
        assert!(!joined.contains(" run "));
        assert!(!joined.contains("--rm"));
    }

    #[test]
    fn bounded_reader_keeps_only_the_tail() {
        let input = b"0123456789";
        let (output, truncated) = read_tail_bounded(input.as_slice(), 4).unwrap();
        assert_eq!(output, b"6789");
        assert!(truncated);
    }

    #[test]
    fn logs_are_redacted_by_line() {
        let logs = DockerContainerLogs::new(
            "safe line\nAuthorization: Bearer abc\ncredential=jobcred_example\ndone",
            "",
            false,
            false,
        );
        assert_eq!(
            logs.stdout_tail(),
            "safe line\n[REDACTED]\n[REDACTED]\ndone"
        );
        assert!(!logs.stdout_tail().contains("abc"));
        assert!(!logs.stdout_tail().contains("jobcred_example"));
    }

    #[test]
    fn public_log_constructor_enforces_byte_limit() {
        let logs =
            DockerContainerLogs::new(&"x".repeat(MAX_DOCKER_LOG_BYTES + 1), "", false, false);
        assert!(logs.stdout_tail().len() <= MAX_DOCKER_LOG_BYTES);
        assert!(logs.stdout_tail().is_empty());
        assert!(logs.stdout_truncated());
    }

    #[test]
    fn docker_state_is_minimal_and_typed() {
        let state: DockerStateOutput = serde_json::from_str(
            r#"{"Running":false,"ExitCode":137,"OOMKilled":true,"StartedAt":"2026-08-07T00:00:00Z","FinishedAt":"2026-08-07T00:00:01Z"}"#,
        )
        .unwrap();
        assert!(!state.running);
        assert_eq!(state.exit_code, 137);
        assert!(state.oom_killed);
    }
}
