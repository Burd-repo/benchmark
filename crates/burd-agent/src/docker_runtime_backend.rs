use crate::provider_job_executor::JobCancellation;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub const MAX_DOCKER_LOG_BYTES: usize = 64 * 1024;
pub const MAX_DOCKER_LOG_LINES: usize = 200;
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(10);
const COMMAND_REAP_TIMEOUT: Duration = Duration::from_secs(2);
const ARTIFACT_HELPER_POLL_INTERVAL: Duration = Duration::from_millis(25);
const ARTIFACT_HELPER_CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const ARTIFACT_VOLUME_OVERHEAD_BYTES: u64 = 1024 * 1024;
const ARTIFACT_HELPER_MEMORY_MIB: u64 = 256;
const ARTIFACT_HELPER_PIDS: u32 = 32;
const ARTIFACT_HELPER_CPUS: &str = "1";
const ARTIFACT_INPUT_PATH: &str = "/burd/input";
const ARTIFACT_OUTPUT_PATH: &str = "/burd/output";
const HELPER_STAGING_PATH: &str = "/burd/staging";
const HELPER_VOLUME_PATH: &str = "/burd/volume";

#[derive(Clone)]
pub struct DockerCommandControl {
    deadline: Instant,
    cancellation: Option<JobCancellation>,
}

impl DockerCommandControl {
    pub fn cancellable(timeout: Duration, cancellation: JobCancellation) -> Self {
        Self::new(timeout, Some(cancellation))
    }

    pub fn cleanup(timeout: Duration) -> Self {
        Self::new(timeout, None)
    }

    fn new(timeout: Duration, cancellation: Option<JobCancellation>) -> Self {
        let now = Instant::now();
        Self {
            deadline: now.checked_add(timeout).unwrap_or(now),
            cancellation,
        }
    }

    fn interruption(&self) -> Option<DockerRuntimeError> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(JobCancellation::requested)
        {
            Some(DockerRuntimeError::new("runtime_command_cancelled"))
        } else if Instant::now() >= self.deadline {
            Some(DockerRuntimeError::new("runtime_command_timed_out"))
        } else {
            None
        }
    }

    fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

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
    pub artifact_workspace: bool,
    pub input_artifact_count: u32,
    pub output_artifact_count: u32,
    pub input_artifact_bytes: u64,
    pub output_artifact_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExistingDockerContainer {
    pub labels: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerVolumeInspect {
    driver: String,
    options: Option<BTreeMap<String, String>>,
    labels: Option<BTreeMap<String, String>>,
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
    fn verify_environment(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError>;

    fn existing_container(
        &self,
        name: &str,
        control: &DockerCommandControl,
    ) -> Result<Option<ExistingDockerContainer>, DockerRuntimeError>;

    fn create(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<String, DockerRuntimeError>;
    fn start(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError>;
    fn prepare_artifacts(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError>;
    fn stage_inputs(
        &self,
        plan: &DockerContainerPlan,
        inputs_dir: &Path,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError>;
    fn collect_outputs(
        &self,
        plan: &DockerContainerPlan,
        outputs_dir: &Path,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError>;
    fn cleanup_artifacts(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError>;
    fn inspect(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<DockerContainerState, DockerRuntimeError>;

    /// Returns already bounded and redacted tails; implementations must never expose raw logs.
    fn logs(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<DockerContainerLogs, DockerRuntimeError>;
    fn terminate(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError>;
    fn kill(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError>;
    fn remove(
        &self,
        container_id_or_name: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError>;
}

#[derive(Clone, Debug)]
struct DockerCliRuntime {
    docker_program: String,
    nvidia_smi_program: String,
}

impl Default for DockerCliRuntime {
    fn default() -> Self {
        Self {
            docker_program: "docker".to_string(),
            nvidia_smi_program: "nvidia-smi".to_string(),
        }
    }
}

impl DockerCliRuntime {
    fn docker(
        &self,
        args: &[String],
        control: &DockerCommandControl,
    ) -> Result<BoundedCommandOutput, DockerRuntimeError> {
        run_bounded_command(&self.docker_program, args, control)
    }

    fn successful_docker(
        &self,
        args: &[String],
        code: &'static str,
        control: &DockerCommandControl,
    ) -> Result<BoundedCommandOutput, DockerRuntimeError> {
        let output = self.docker(args, control)?;
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
        ]);
        if plan.artifact_workspace {
            let input_volume = artifact_volume_name(plan, "input");
            let output_volume = artifact_volume_name(plan, "output");
            args.extend([
                "--mount".to_string(),
                format!(
                    "type=volume,source={input_volume},destination={ARTIFACT_INPUT_PATH},readonly,volume-nocopy"
                ),
                "--mount".to_string(),
                format!(
                    "type=volume,source={output_volume},destination={ARTIFACT_OUTPUT_PATH},volume-nocopy"
                ),
            ]);
        }
        args.push(plan.image_ref.clone());
        args
    }

    fn verify_artifact_helper_image(
        &self,
        helper_image_ref: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        if !is_immutable_image_ref(helper_image_ref) {
            return Err(DockerRuntimeError::new("artifact_helper_image_invalid"));
        }
        let image = self.successful_docker(
            &[
                "image".into(),
                "inspect".into(),
                "--format".into(),
                "{{.Id}}".into(),
                helper_image_ref.to_string(),
            ],
            "artifact_helper_image_unavailable",
            control,
        )?;
        if image.stdout.trim().is_empty() {
            Err(DockerRuntimeError::new("artifact_helper_image_invalid"))
        } else {
            Ok(())
        }
    }

    fn prepare_artifact_storage(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        if !plan.artifact_workspace {
            return Ok(());
        }
        self.remove_owned_helper(plan, "import", control)?;
        self.remove_owned_helper(plan, "export", control)?;
        self.remove_owned_volume(plan, "input", control)?;
        self.remove_owned_volume(plan, "output", control)?;
        self.create_artifact_volume(plan, "input", control)?;
        if let Err(error) = self.create_artifact_volume(plan, "output", control) {
            let _ = self.remove_owned_volume(
                plan,
                "input",
                &DockerCommandControl::cleanup(ARTIFACT_HELPER_CLEANUP_TIMEOUT),
            );
            return Err(error);
        }
        Ok(())
    }

    fn stage_inputs(
        &self,
        plan: &DockerContainerPlan,
        helper_image_ref: &str,
        inputs_dir: &Path,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        if plan.input_artifact_count == 0 {
            return Ok(());
        }
        let helper_id = self.create_artifact_helper(plan, helper_image_ref, "import", control)?;
        let source = inputs_dir.join(".").display().to_string();
        let copy = self.successful_docker(
            &[
                "cp".into(),
                source,
                format!("{helper_id}:{HELPER_STAGING_PATH}"),
            ],
            "artifact_helper_input_stage_failed",
            control,
        );
        let result = copy.and_then(|_| self.run_artifact_helper(&helper_id, control));
        self.finish_artifact_helper(&helper_id, result)
    }

    fn collect_outputs(
        &self,
        plan: &DockerContainerPlan,
        helper_image_ref: &str,
        outputs_dir: &Path,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        if plan.output_artifact_count == 0 {
            return Ok(());
        }
        let helper_id = self.create_artifact_helper(plan, helper_image_ref, "export", control)?;
        let result = self.run_artifact_helper(&helper_id, control).and_then(|_| {
            self.successful_docker(
                &[
                    "cp".into(),
                    format!("{helper_id}:{HELPER_STAGING_PATH}/."),
                    outputs_dir.display().to_string(),
                ],
                "artifact_helper_output_collect_failed",
                control,
            )?;
            Ok(())
        });
        self.finish_artifact_helper(&helper_id, result)
    }

    fn cleanup_artifact_storage(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        if !plan.artifact_workspace {
            return Ok(());
        }
        let mut first_error = None;
        for role in ["import", "export"] {
            if let Err(error) = self.remove_owned_helper(plan, role, control) {
                first_error.get_or_insert(error);
            }
        }
        for role in ["input", "output"] {
            if let Err(error) = self.remove_owned_volume(plan, role, control) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn create_artifact_volume(
        &self,
        plan: &DockerContainerPlan,
        role: &'static str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        let name = artifact_volume_name(plan, role);
        let total = if role == "input" {
            plan.input_artifact_bytes
        } else {
            plan.output_artifact_bytes
        };
        let (uid, gid, mode) = if role == "input" {
            (0, 0, "0755")
        } else {
            (1000, 1000, "0700")
        };
        let mount_options = format!(
            "size={},uid={uid},gid={gid},mode={mode},noexec,nosuid,nodev",
            artifact_volume_capacity(total)
        );
        let mut args = vec![
            "volume".into(),
            "create".into(),
            "--driver".into(),
            "local".into(),
            "--opt".into(),
            "type=tmpfs".into(),
            "--opt".into(),
            "device=tmpfs".into(),
            "--opt".into(),
            format!("o={mount_options}"),
        ];
        for (key, value) in artifact_labels(plan, role) {
            args.push("--label".into());
            args.push(format!("{key}={value}"));
        }
        args.push(name.clone());
        let output = self.successful_docker(&args, "artifact_volume_create_failed", control)?;
        if output.stdout_truncated || output.stdout.trim() != name {
            return Err(DockerRuntimeError::new("artifact_volume_create_invalid"));
        }
        let created = self
            .existing_volume(&name, control)?
            .ok_or_else(|| DockerRuntimeError::new("artifact_volume_create_invalid"))?;
        let options = created.options.unwrap_or_default();
        if created.driver != "local"
            || options.get("type").map(String::as_str) != Some("tmpfs")
            || options.get("device").map(String::as_str) != Some("tmpfs")
            || options.get("o") != Some(&mount_options)
            || !labels_contain(
                &created.labels.unwrap_or_default(),
                &artifact_labels(plan, role),
            )
        {
            return Err(DockerRuntimeError::new("artifact_volume_create_invalid"));
        }
        Ok(())
    }

    fn create_artifact_helper(
        &self,
        plan: &DockerContainerPlan,
        helper_image_ref: &str,
        role: &'static str,
        control: &DockerCommandControl,
    ) -> Result<String, DockerRuntimeError> {
        let output = self.successful_docker(
            &artifact_helper_create_args(plan, helper_image_ref, role),
            "artifact_helper_create_failed",
            control,
        )?;
        parse_container_id(&output)
    }

    fn run_artifact_helper(
        &self,
        helper_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.start(helper_id, control)?;
        loop {
            let state = self.inspect(helper_id, control)?;
            if !state.running {
                if state.exit_code == Some(0) && !state.oom_killed {
                    return Ok(());
                }
                #[cfg(test)]
                if let Ok(logs) = self.logs(helper_id, control) {
                    eprintln!("artifact helper stdout: {}", logs.stdout_tail());
                    eprintln!("artifact helper stderr: {}", logs.stderr_tail());
                }
                return Err(DockerRuntimeError::new("artifact_helper_failed"));
            }
            if control.remaining().is_zero() {
                return Err(DockerRuntimeError::new("runtime_command_timed_out"));
            }
            thread::sleep(ARTIFACT_HELPER_POLL_INTERVAL.min(control.remaining()));
        }
    }

    fn finish_artifact_helper(
        &self,
        helper_id: &str,
        result: Result<(), DockerRuntimeError>,
    ) -> Result<(), DockerRuntimeError> {
        let cleanup = self.remove_container_without_volumes(
            helper_id,
            &DockerCommandControl::cleanup(ARTIFACT_HELPER_CLEANUP_TIMEOUT),
        );
        match (result, cleanup) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(_)) => Err(DockerRuntimeError::new("artifact_helper_cleanup_failed")),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn remove_container_without_volumes(
        &self,
        container_id_or_name: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.successful_docker(
            &["rm".into(), "--force".into(), container_id_or_name.into()],
            "artifact_helper_cleanup_failed",
            control,
        )?;
        Ok(())
    }

    fn remove_owned_helper(
        &self,
        plan: &DockerContainerPlan,
        role: &'static str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        let name = artifact_helper_name(plan, role);
        let Some(existing) = self.existing_container(&name, control)? else {
            return Ok(());
        };
        if !labels_contain(&existing.labels, &artifact_labels(plan, role)) {
            return Err(DockerRuntimeError::new("artifact_helper_name_conflict"));
        }
        self.remove_container_without_volumes(&name, control)
    }

    fn remove_owned_volume(
        &self,
        plan: &DockerContainerPlan,
        role: &'static str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        let name = artifact_volume_name(plan, role);
        let Some(volume) = self.existing_volume(&name, control)? else {
            return Ok(());
        };
        if !labels_contain(
            &volume.labels.unwrap_or_default(),
            &artifact_labels(plan, role),
        ) {
            return Err(DockerRuntimeError::new("artifact_volume_name_conflict"));
        }
        self.successful_docker(
            &["volume".into(), "rm".into(), name],
            "artifact_volume_remove_failed",
            control,
        )?;
        Ok(())
    }

    fn existing_volume(
        &self,
        name: &str,
        control: &DockerCommandControl,
    ) -> Result<Option<DockerVolumeInspect>, DockerRuntimeError> {
        let listing = self.successful_docker(
            &[
                "volume".into(),
                "ls".into(),
                "--filter".into(),
                format!("name={name}"),
                "--format".into(),
                "{{.Name}}".into(),
            ],
            "artifact_volume_list_failed",
            control,
        )?;
        if listing.stdout_truncated {
            return Err(DockerRuntimeError::new("artifact_volume_list_invalid"));
        }
        let exact = listing
            .stdout
            .lines()
            .map(str::trim)
            .filter(|candidate| *candidate == name)
            .count();
        if exact == 0 {
            return Ok(None);
        }
        if exact != 1 {
            return Err(DockerRuntimeError::new("artifact_volume_list_invalid"));
        }
        let output = self.successful_docker(
            &[
                "volume".into(),
                "inspect".into(),
                "--format".into(),
                "{{json .}}".into(),
                name.to_string(),
            ],
            "artifact_volume_inspect_failed",
            control,
        )?;
        serde_json::from_str::<DockerVolumeInspect>(output.stdout.trim())
            .map(Some)
            .map_err(|_| DockerRuntimeError::new("artifact_volume_labels_invalid"))
    }
}

impl DockerCliRuntime {
    fn verify_linux_container_environment(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.verify_linux_docker_server(control)?;
        self.verify_nvidia_gpu_and_image(plan, control)
    }

    fn verify_linux_docker_server(
        &self,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        let server_os = self.successful_docker(
            &["version".into(), "--format".into(), "{{.Server.Os}}".into()],
            "docker_unavailable",
            control,
        )?;
        if server_os.stdout.trim() != "linux" {
            return Err(DockerRuntimeError::new("linux_container_engine_required"));
        }
        Ok(())
    }

    fn verify_wsl2_docker_kernel(
        &self,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        let kernel = self.successful_docker(
            &[
                "info".into(),
                "--format".into(),
                "{{.KernelVersion}}".into(),
            ],
            "docker_runtime_probe_failed",
            control,
        )?;
        if kernel.stdout_truncated || !is_wsl2_kernel_version(&kernel.stdout) {
            return Err(DockerRuntimeError::new("wsl2_runtime_unavailable"));
        }
        Ok(())
    }

    fn verify_nvidia_gpu_and_image(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        let runtimes = self.successful_docker(
            &[
                "info".into(),
                "--format".into(),
                "{{json .Runtimes}}".into(),
            ],
            "docker_runtime_probe_failed",
            control,
        )?;
        let runtime_map: serde_json::Value = serde_json::from_str(runtimes.stdout.trim())
            .map_err(|_| DockerRuntimeError::new("docker_runtime_probe_invalid"))?;
        if runtime_map.get("nvidia").is_none() {
            return Err(DockerRuntimeError::new("nvidia_runtime_unavailable"));
        }

        let gpu_output = run_bounded_command(
            &self.nvidia_smi_program,
            &["--query-gpu=uuid".into(), "--format=csv,noheader".into()],
            control,
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
            control,
        )?;
        if image.stdout.trim().is_empty() {
            return Err(DockerRuntimeError::new("container_image_invalid"));
        }
        Ok(())
    }

    fn existing_container(
        &self,
        name: &str,
        control: &DockerCommandControl,
    ) -> Result<Option<ExistingDockerContainer>, DockerRuntimeError> {
        let listing = self.successful_docker(
            &[
                "container".into(),
                "ls".into(),
                "--all".into(),
                "--filter".into(),
                format!("name={name}"),
                "--format".into(),
                "{{.ID}}\t{{.Names}}".into(),
            ],
            "container_list_failed",
            control,
        )?;
        let Some(container_id) =
            exact_container_id(&listing.stdout, listing.stdout_truncated, name)?
        else {
            return Ok(None);
        };
        let output = self.successful_docker(
            &[
                "container".into(),
                "inspect".into(),
                "--format".into(),
                "{{json .Config.Labels}}".into(),
                container_id,
            ],
            "container_inspect_failed",
            control,
        )?;
        let labels = serde_json::from_str::<Option<BTreeMap<String, String>>>(output.stdout.trim())
            .map_err(|_| DockerRuntimeError::new("container_labels_invalid"))?
            .unwrap_or_default();
        Ok(Some(ExistingDockerContainer { labels }))
    }

    fn create(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<String, DockerRuntimeError> {
        let output =
            self.successful_docker(&Self::create_args(plan), "container_create_failed", control)?;
        parse_container_id(&output)
    }

    fn start(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.successful_docker(
            &["start".into(), container_id.to_string()],
            "container_start_failed",
            control,
        )?;
        Ok(())
    }

    fn inspect(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<DockerContainerState, DockerRuntimeError> {
        let output = self.successful_docker(
            &[
                "container".into(),
                "inspect".into(),
                "--format".into(),
                "{{json .State}}".into(),
                container_id.to_string(),
            ],
            "container_inspect_failed",
            control,
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

    fn logs(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<DockerContainerLogs, DockerRuntimeError> {
        let output = self.successful_docker(
            &[
                "logs".into(),
                "--tail".into(),
                MAX_DOCKER_LOG_LINES.to_string(),
                container_id.to_string(),
            ],
            "container_logs_failed",
            control,
        )?;
        Ok(DockerContainerLogs::new(
            &output.stdout,
            &output.stderr,
            output.stdout_truncated,
            output.stderr_truncated,
        ))
    }

    fn terminate(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.successful_docker(
            &[
                "kill".into(),
                "--signal".into(),
                "TERM".into(),
                container_id.to_string(),
            ],
            "container_terminate_failed",
            control,
        )?;
        Ok(())
    }

    fn kill(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.successful_docker(
            &[
                "kill".into(),
                "--signal".into(),
                "KILL".into(),
                container_id.to_string(),
            ],
            "container_kill_failed",
            control,
        )?;
        Ok(())
    }

    fn remove(
        &self,
        container_id_or_name: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.successful_docker(
            &[
                "rm".into(),
                "--force".into(),
                "--volumes".into(),
                container_id_or_name.to_string(),
            ],
            "container_remove_failed",
            control,
        )?;
        Ok(())
    }
}

fn parse_container_id(output: &BoundedCommandOutput) -> Result<String, DockerRuntimeError> {
    let id = output.stdout.trim();
    if output.stdout_truncated
        || !(12..=64).contains(&id.len())
        || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DockerRuntimeError::new("container_id_invalid"));
    }
    Ok(id.to_string())
}

fn is_immutable_image_ref(image_ref: &str) -> bool {
    let digest = image_ref
        .strip_prefix("sha256:")
        .or_else(|| image_ref.rsplit_once("@sha256:").map(|(_, digest)| digest));
    digest.is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn artifact_volume_capacity(content_bytes: u64) -> u64 {
    content_bytes
        .saturating_add(ARTIFACT_VOLUME_OVERHEAD_BYTES)
        .max(ARTIFACT_VOLUME_OVERHEAD_BYTES)
}

fn artifact_volume_name(plan: &DockerContainerPlan, role: &str) -> String {
    format!("{}-artifact-{role}", plan.name)
}

fn artifact_helper_name(plan: &DockerContainerPlan, role: &str) -> String {
    format!("{}-helper-{role}", plan.name)
}

fn artifact_labels(plan: &DockerContainerPlan, role: &str) -> BTreeMap<String, String> {
    let mut labels = plan.labels.clone();
    labels.insert("com.burd.artifact_role".to_string(), role.to_string());
    labels
}

fn labels_contain(actual: &BTreeMap<String, String>, expected: &BTreeMap<String, String>) -> bool {
    expected
        .iter()
        .all(|(key, value)| actual.get(key) == Some(value))
}

fn artifact_helper_create_args(
    plan: &DockerContainerPlan,
    helper_image_ref: &str,
    role: &'static str,
) -> Vec<String> {
    let (user, volume_role, readonly_volume, maximum_bytes, maximum_files) = if role == "import" {
        (
            "0:0",
            "input",
            false,
            plan.input_artifact_bytes,
            plan.input_artifact_count,
        )
    } else {
        (
            "1000:1000",
            "output",
            true,
            plan.output_artifact_bytes,
            plan.output_artifact_count,
        )
    };
    let mut args = vec![
        "create".into(),
        "--pull".into(),
        "never".into(),
        "--name".into(),
        artifact_helper_name(plan, role),
        "--restart".into(),
        "no".into(),
        "--no-healthcheck".into(),
    ];
    for (key, value) in artifact_labels(plan, role) {
        args.push("--label".into());
        args.push(format!("{key}={value}"));
    }
    args.extend([
        "--user".into(),
        user.into(),
        "--cap-drop".into(),
        "ALL".into(),
        "--security-opt".into(),
        "no-new-privileges".into(),
        "--security-opt".into(),
        "seccomp=builtin".into(),
        "--network".into(),
        "none".into(),
        "--ipc".into(),
        "none".into(),
        "--pids-limit".into(),
        ARTIFACT_HELPER_PIDS.to_string(),
        "--memory".into(),
        format!("{ARTIFACT_HELPER_MEMORY_MIB}m"),
        "--memory-swap".into(),
        format!("{ARTIFACT_HELPER_MEMORY_MIB}m"),
        "--cpus".into(),
        ARTIFACT_HELPER_CPUS.into(),
    ]);
    if role == "import" {
        // docker cp can retain a restrictive source owner/mode. This narrow
        // capability lets the trusted helper read only paths already visible
        // in its isolated container namespace; no host path is mounted.
        args.extend(["--cap-add".into(), "DAC_READ_SEARCH".into()]);
    }
    // Docker copies staging bytes through the helper's ephemeral container layer.
    // That layer must remain writable until the helper is removed; the workload
    // rootfs is still read-only and never receives a host path.
    let readonly = if readonly_volume { ",readonly" } else { "" };
    args.extend([
        "--mount".into(),
        format!(
            "type=volume,source={},destination={HELPER_VOLUME_PATH}{readonly},volume-nocopy",
            artifact_volume_name(plan, volume_role)
        ),
        helper_image_ref.to_string(),
        role.to_string(),
        maximum_bytes.to_string(),
        maximum_files.to_string(),
    ]);
    args
}

#[derive(Clone, Debug, Default)]
pub struct LinuxNativeDockerBackend {
    cli: DockerCliRuntime,
    artifact_helper_image_ref: Option<String>,
}

impl LinuxNativeDockerBackend {
    pub fn create_args(plan: &DockerContainerPlan) -> Vec<String> {
        DockerCliRuntime::create_args(plan)
    }

    pub fn with_artifact_helper_image(
        image_ref: impl Into<String>,
    ) -> Result<Self, DockerRuntimeError> {
        let image_ref = image_ref.into();
        if !is_immutable_image_ref(&image_ref) {
            return Err(DockerRuntimeError::new("artifact_helper_image_invalid"));
        }
        Ok(Self {
            cli: DockerCliRuntime::default(),
            artifact_helper_image_ref: Some(image_ref),
        })
    }

    fn artifact_helper_image(&self) -> Result<&str, DockerRuntimeError> {
        self.artifact_helper_image_ref
            .as_deref()
            .ok_or_else(|| DockerRuntimeError::new("artifact_helper_not_configured"))
    }
}

impl DockerRuntimeBackend for LinuxNativeDockerBackend {
    fn runtime_backend(&self) -> &'static str {
        "docker_linux_native"
    }

    fn verify_environment(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        if std::env::consts::OS != "linux" {
            return Err(DockerRuntimeError::new("linux_native_host_required"));
        }
        self.cli.verify_linux_container_environment(plan, control)?;
        if plan.artifact_workspace {
            self.cli
                .verify_artifact_helper_image(self.artifact_helper_image()?, control)?;
        }
        Ok(())
    }

    fn existing_container(
        &self,
        name: &str,
        control: &DockerCommandControl,
    ) -> Result<Option<ExistingDockerContainer>, DockerRuntimeError> {
        self.cli.existing_container(name, control)
    }

    fn create(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<String, DockerRuntimeError> {
        self.cli.create(plan, control)
    }

    fn start(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.start(container_id, control)
    }

    fn prepare_artifacts(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.prepare_artifact_storage(plan, control)
    }

    fn stage_inputs(
        &self,
        plan: &DockerContainerPlan,
        inputs_dir: &Path,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli
            .stage_inputs(plan, self.artifact_helper_image()?, inputs_dir, control)
    }

    fn collect_outputs(
        &self,
        plan: &DockerContainerPlan,
        outputs_dir: &Path,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli
            .collect_outputs(plan, self.artifact_helper_image()?, outputs_dir, control)
    }

    fn cleanup_artifacts(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.cleanup_artifact_storage(plan, control)
    }

    fn inspect(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<DockerContainerState, DockerRuntimeError> {
        self.cli.inspect(container_id, control)
    }

    fn logs(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<DockerContainerLogs, DockerRuntimeError> {
        self.cli.logs(container_id, control)
    }

    fn terminate(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.terminate(container_id, control)
    }

    fn kill(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.kill(container_id, control)
    }

    fn remove(
        &self,
        container_id_or_name: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.remove(container_id_or_name, control)
    }
}

#[derive(Clone, Debug)]
pub struct WindowsWsl2DockerBackend {
    cli: DockerCliRuntime,
    wsl_program: String,
    artifact_helper_image_ref: Option<String>,
}

impl Default for WindowsWsl2DockerBackend {
    fn default() -> Self {
        Self {
            cli: DockerCliRuntime::default(),
            wsl_program: "wsl.exe".to_string(),
            artifact_helper_image_ref: None,
        }
    }
}

impl WindowsWsl2DockerBackend {
    pub fn create_args(plan: &DockerContainerPlan) -> Vec<String> {
        DockerCliRuntime::create_args(plan)
    }

    fn wsl_kernel_args() -> [String; 3] {
        ["--system".into(), "uname".into(), "-r".into()]
    }

    pub fn with_artifact_helper_image(
        image_ref: impl Into<String>,
    ) -> Result<Self, DockerRuntimeError> {
        let image_ref = image_ref.into();
        if !is_immutable_image_ref(&image_ref) {
            return Err(DockerRuntimeError::new("artifact_helper_image_invalid"));
        }
        Ok(Self {
            cli: DockerCliRuntime::default(),
            wsl_program: "wsl.exe".to_string(),
            artifact_helper_image_ref: Some(image_ref),
        })
    }

    fn artifact_helper_image(&self) -> Result<&str, DockerRuntimeError> {
        self.artifact_helper_image_ref
            .as_deref()
            .ok_or_else(|| DockerRuntimeError::new("artifact_helper_not_configured"))
    }
}

impl DockerRuntimeBackend for WindowsWsl2DockerBackend {
    fn runtime_backend(&self) -> &'static str {
        "docker_wsl2"
    }

    fn verify_environment(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        if std::env::consts::OS != "windows" {
            return Err(DockerRuntimeError::new("windows_wsl2_host_required"));
        }
        let kernel = run_bounded_command(&self.wsl_program, &Self::wsl_kernel_args(), control)?;
        if !kernel.status.success() {
            return Err(DockerRuntimeError::new("wsl2_unavailable"));
        }
        if kernel.stdout_truncated || !is_wsl2_kernel_version(&kernel.stdout) {
            return Err(DockerRuntimeError::new("wsl2_runtime_unavailable"));
        }
        self.cli.verify_linux_docker_server(control)?;
        self.cli.verify_wsl2_docker_kernel(control)?;
        self.cli.verify_nvidia_gpu_and_image(plan, control)?;
        if plan.artifact_workspace {
            self.cli
                .verify_artifact_helper_image(self.artifact_helper_image()?, control)?;
        }
        Ok(())
    }

    fn existing_container(
        &self,
        name: &str,
        control: &DockerCommandControl,
    ) -> Result<Option<ExistingDockerContainer>, DockerRuntimeError> {
        self.cli.existing_container(name, control)
    }

    fn create(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<String, DockerRuntimeError> {
        self.cli.create(plan, control)
    }

    fn start(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.start(container_id, control)
    }

    fn prepare_artifacts(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.prepare_artifact_storage(plan, control)
    }

    fn stage_inputs(
        &self,
        plan: &DockerContainerPlan,
        inputs_dir: &Path,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli
            .stage_inputs(plan, self.artifact_helper_image()?, inputs_dir, control)
    }

    fn collect_outputs(
        &self,
        plan: &DockerContainerPlan,
        outputs_dir: &Path,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli
            .collect_outputs(plan, self.artifact_helper_image()?, outputs_dir, control)
    }

    fn cleanup_artifacts(
        &self,
        plan: &DockerContainerPlan,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.cleanup_artifact_storage(plan, control)
    }

    fn inspect(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<DockerContainerState, DockerRuntimeError> {
        self.cli.inspect(container_id, control)
    }

    fn logs(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<DockerContainerLogs, DockerRuntimeError> {
        self.cli.logs(container_id, control)
    }

    fn terminate(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.terminate(container_id, control)
    }

    fn kill(
        &self,
        container_id: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.kill(container_id, control)
    }

    fn remove(
        &self,
        container_id_or_name: &str,
        control: &DockerCommandControl,
    ) -> Result<(), DockerRuntimeError> {
        self.cli.remove(container_id_or_name, control)
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

fn is_wsl2_kernel_version(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    !value.is_empty() && (value.contains("microsoft-standard-wsl2") || value.contains("wsl2"))
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

fn exact_container_id(
    stdout: &str,
    stdout_truncated: bool,
    expected_name: &str,
) -> Result<Option<String>, DockerRuntimeError> {
    if stdout_truncated {
        return Err(DockerRuntimeError::new("container_list_invalid"));
    }
    let mut exact_id = None;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Some((id, name)) = line.split_once('\t') else {
            return Err(DockerRuntimeError::new("container_list_invalid"));
        };
        if name != expected_name {
            continue;
        }
        if exact_id.is_some()
            || !(12..=64).contains(&id.len())
            || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(DockerRuntimeError::new("container_list_invalid"));
        }
        exact_id = Some(id.to_string());
    }
    Ok(exact_id)
}

fn run_bounded_command(
    program: &str,
    args: &[String],
    control: &DockerCommandControl,
) -> Result<BoundedCommandOutput, DockerRuntimeError> {
    let mut command = Command::new(program);
    command.args(args);
    run_bounded_process(&mut command, control)
}

fn run_bounded_process(
    command: &mut Command,
    control: &DockerCommandControl,
) -> Result<BoundedCommandOutput, DockerRuntimeError> {
    if let Some(error) = control.interruption() {
        return Err(error);
    }
    let mut child = command
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
    let stdout_reader = spawn_bounded_reader(stdout);
    let stderr_reader = spawn_bounded_reader(stderr);
    let status = loop {
        if let Some(error) = control.interruption() {
            break Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(_) => break Err(DockerRuntimeError::new("runtime_command_wait_failed")),
        }
        thread::sleep(COMMAND_POLL_INTERVAL.min(control.remaining()));
    };

    let status = match status {
        Ok(status) => status,
        Err(error) => {
            terminate_command_child(&mut child)?;
            drop(stdout_reader);
            drop(stderr_reader);
            return Err(error);
        }
    };
    let (stdout, stdout_truncated, stderr, stderr_truncated) =
        collect_bounded_output_readers(stdout_reader, stderr_reader, control)?;
    Ok(BoundedCommandOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        stdout_truncated,
        stderr_truncated,
    })
}

fn terminate_command_child(child: &mut Child) -> Result<(), DockerRuntimeError> {
    if child.kill().is_err() {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            _ => {
                return Err(DockerRuntimeError::new(
                    "runtime_command_termination_failed",
                ));
            }
        }
    }
    let reap_deadline = Instant::now()
        .checked_add(COMMAND_REAP_TIMEOUT)
        .unwrap_or_else(Instant::now);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < reap_deadline => {
                thread::sleep(COMMAND_POLL_INTERVAL);
            }
            _ => {
                return Err(DockerRuntimeError::new(
                    "runtime_command_termination_failed",
                ));
            }
        }
    }
}

type BoundedReader = mpsc::Receiver<io::Result<(Vec<u8>, bool)>>;

fn spawn_bounded_reader<R: Read + Send + 'static>(reader: R) -> BoundedReader {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(read_tail_bounded(reader, MAX_DOCKER_LOG_BYTES));
    });
    receiver
}

fn collect_bounded_output_readers(
    stdout_reader: BoundedReader,
    stderr_reader: BoundedReader,
    control: &DockerCommandControl,
) -> Result<(Vec<u8>, bool, Vec<u8>, bool), DockerRuntimeError> {
    let (stdout, stdout_truncated) = receive_bounded_output(
        stdout_reader,
        control,
        "runtime_stdout_reader_failed",
        "runtime_stdout_read_failed",
    )?;
    let (stderr, stderr_truncated) = receive_bounded_output(
        stderr_reader,
        control,
        "runtime_stderr_reader_failed",
        "runtime_stderr_read_failed",
    )?;
    Ok((stdout, stdout_truncated, stderr, stderr_truncated))
}

fn receive_bounded_output(
    receiver: BoundedReader,
    control: &DockerCommandControl,
    disconnected_code: &'static str,
    read_code: &'static str,
) -> Result<(Vec<u8>, bool), DockerRuntimeError> {
    loop {
        if let Some(error) = control.interruption() {
            return Err(error);
        }
        let wait = COMMAND_POLL_INTERVAL.min(control.remaining());
        match receiver.recv_timeout(wait) {
            Ok(output) => return output.map_err(|_| DockerRuntimeError::new(read_code)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(DockerRuntimeError::new(disconnected_code));
            }
        }
    }
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

    fn sleeping_test_process() -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "docker_runtime_backend::tests::bounded_command_test_helper",
            ])
            .env("BURD_RUNTIME_COMMAND_TEST_HELPER", "1");
        command
    }

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
            artifact_workspace: false,
            input_artifact_count: 0,
            output_artifact_count: 0,
            input_artifact_bytes: 0,
            output_artifact_bytes: 0,
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
    fn artifact_plan_uses_docker_managed_storage_without_host_bind_paths() {
        let mut plan = plan();
        plan.artifact_workspace = true;
        plan.input_artifact_count = 1;
        plan.output_artifact_count = 1;
        plan.input_artifact_bytes = 16;
        plan.output_artifact_bytes = 32 * 1024 * 1024;
        let args = LinuxNativeDockerBackend::create_args(&plan);
        let joined = args.join(" ");
        assert!(joined.contains("destination=/burd/input,readonly,volume-nocopy"));
        assert!(joined.contains("destination=/burd/output,volume-nocopy"));
        assert!(!joined.contains("--tmpfs /burd/output"));
        assert!(!joined.contains("type=bind"));
        assert!(!joined.contains("C:\\"));
        assert!(!joined.contains("/mnt/"));
        assert!(!joined.contains("\\\\wsl$"));
    }

    #[test]
    fn artifact_helper_contract_is_fixed_offline_and_digest_pinned() {
        let mut plan = plan();
        plan.artifact_workspace = true;
        plan.input_artifact_count = 1;
        plan.output_artifact_count = 1;
        plan.input_artifact_bytes = 16;
        plan.output_artifact_bytes = 32;
        let image = format!("burd/artifact-helper@sha256:{}", "b".repeat(64));
        let import = artifact_helper_create_args(&plan, &image, "import");
        let export = artifact_helper_create_args(&plan, &image, "export");
        let import_joined = import.join(" ");
        let export_joined = export.join(" ");

        for args in [&import, &export] {
            let joined = args.join(" ");
            for required in [
                "--pull",
                "never",
                "--cap-drop",
                "ALL",
                "no-new-privileges",
                "seccomp=builtin",
                "--network",
                "none",
                "--ipc",
            ] {
                assert!(args.iter().any(|argument| argument == required));
            }
            assert!(joined.contains(&image));
            assert!(!joined.contains("type=bind"));
            assert!(!joined.contains("docker.sock"));
            assert!(!joined.contains("sh -c"));
            assert!(!joined.contains("--privileged"));
            assert!(!args.iter().any(|argument| argument == "--read-only"));
        }
        assert!(import_joined.contains("--user 0:0"));
        assert!(import_joined.contains("--cap-add DAC_READ_SEARCH"));
        assert!(import_joined.contains("-artifact-input"));
        assert!(!import_joined.contains("destination=/burd/volume,readonly"));
        assert!(export_joined.contains("--user 1000:1000"));
        assert!(!export_joined.contains("--cap-add"));
        assert!(export_joined.contains("-artifact-output"));
        assert!(export_joined.contains("destination=/burd/volume,readonly"));
        assert!(is_immutable_image_ref(&image));
        assert!(is_immutable_image_ref(&format!(
            "sha256:{}",
            "c".repeat(64)
        )));
        assert!(!is_immutable_image_ref("burd/artifact-helper:latest"));
    }

    #[test]
    fn linux_and_windows_backends_share_the_exact_container_contract() {
        let linux = LinuxNativeDockerBackend::create_args(&plan());
        let windows = WindowsWsl2DockerBackend::create_args(&plan());

        assert_eq!(windows, linux);
        assert_eq!(
            LinuxNativeDockerBackend::default().runtime_backend(),
            "docker_linux_native"
        );
        assert_eq!(
            WindowsWsl2DockerBackend::default().runtime_backend(),
            "docker_wsl2"
        );
    }

    #[test]
    fn windows_backend_has_no_windows_or_wsl_filesystem_mounts() {
        let joined = WindowsWsl2DockerBackend::create_args(&plan()).join(" ");
        for forbidden in [
            "C:\\", "c:\\", "/mnt/", "\\\\wsl$", "--mount", "--volume", "-v ",
        ] {
            assert!(!joined.contains(forbidden), "found {forbidden}");
        }
    }

    #[test]
    fn windows_wsl_probe_is_structured_and_requires_a_wsl2_kernel() {
        assert_eq!(
            WindowsWsl2DockerBackend::wsl_kernel_args(),
            ["--system", "uname", "-r"]
        );
        assert!(is_wsl2_kernel_version(
            "6.6.87.2-microsoft-standard-WSL2\r\n"
        ));
        assert!(is_wsl2_kernel_version("5.15.90-wsl2-custom"));
        assert!(!is_wsl2_kernel_version("6.8.0-linuxkit"));
        assert!(!is_wsl2_kernel_version(""));

        let command = WindowsWsl2DockerBackend::wsl_kernel_args().join(" ");
        assert!(!command.contains("sh"));
        assert!(!command.contains("-c"));
    }

    #[test]
    fn platform_backends_reject_the_wrong_host_before_runtime_probes() {
        let control = DockerCommandControl::cleanup(Duration::from_secs(1));
        if std::env::consts::OS == "windows" {
            let error = LinuxNativeDockerBackend::default()
                .verify_environment(&plan(), &control)
                .err()
                .unwrap();
            assert_eq!(error.code(), "linux_native_host_required");
        } else {
            let error = WindowsWsl2DockerBackend::default()
                .verify_environment(&plan(), &control)
                .err()
                .unwrap();
            assert_eq!(error.code(), "windows_wsl2_host_required");
        }
    }

    #[test]
    #[ignore = "requires Docker and a locally available digest-pinned burd-artifact-helper image; NVIDIA is not required"]
    fn physical_docker_artifact_volume_bridge_roundtrip_without_nvidia() {
        use burd_protocol::{create_private_directory_all, create_private_file_new};
        use std::io::Write;

        let helper_image = std::env::var("BURD_ARTIFACT_HELPER_TEST_IMAGE")
            .expect("BURD_ARTIFACT_HELPER_TEST_IMAGE is required");
        assert!(is_immutable_image_ref(&helper_image));
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("burd-artifact-bridge-{unique}"));
        let inputs = root.join("inputs");
        let outputs = root.join("outputs");
        create_private_directory_all(&inputs).unwrap();
        create_private_directory_all(&outputs).unwrap();
        let input_bytes = b"hello burd";
        let mut input = create_private_file_new(&inputs.join("input.bin")).unwrap();
        input.write_all(input_bytes).unwrap();
        input.sync_all().unwrap();
        drop(input);

        let mut plan = plan();
        plan.name = format!("burd-artifact-gate-{unique}");
        plan.image_ref = helper_image.clone();
        plan.artifact_workspace = true;
        plan.input_artifact_count = 1;
        plan.output_artifact_count = 1;
        plan.input_artifact_bytes = input_bytes.len() as u64;
        plan.output_artifact_bytes = (input_bytes.len() + b"burd:".len()) as u64;
        let cli = DockerCliRuntime::default();
        let control = DockerCommandControl::cleanup(Duration::from_secs(120));
        cli.verify_artifact_helper_image(&helper_image, &control)
            .unwrap();
        cli.prepare_artifact_storage(&plan, &control).unwrap();

        let workload_name = format!("{}-roundtrip", plan.name);
        let workload_args = vec![
            "create".into(),
            "--pull".into(),
            "never".into(),
            "--name".into(),
            workload_name.clone(),
            "--user".into(),
            "1000:1000".into(),
            "--read-only".into(),
            "--cap-drop".into(),
            "ALL".into(),
            "--security-opt".into(),
            "no-new-privileges".into(),
            "--security-opt".into(),
            "seccomp=builtin".into(),
            "--network".into(),
            "none".into(),
            "--ipc".into(),
            "none".into(),
            "--mount".into(),
            format!(
                "type=volume,source={},destination={ARTIFACT_INPUT_PATH},readonly,volume-nocopy",
                artifact_volume_name(&plan, "input")
            ),
            "--mount".into(),
            format!(
                "type=volume,source={},destination={ARTIFACT_OUTPUT_PATH},volume-nocopy",
                artifact_volume_name(&plan, "output")
            ),
            helper_image.clone(),
            "roundtrip-test".into(),
        ];

        let result = (|| {
            cli.stage_inputs(&plan, &helper_image, &inputs, &control)?;
            let created = cli.successful_docker(
                &workload_args,
                "artifact_gate_container_create_failed",
                &control,
            )?;
            let workload_id = parse_container_id(&created)?;
            let execution = cli.run_artifact_helper(&workload_id, &control);
            let collection = execution
                .and_then(|_| cli.collect_outputs(&plan, &helper_image, &outputs, &control));
            let removal = cli.remove_container_without_volumes(
                &workload_id,
                &DockerCommandControl::cleanup(Duration::from_secs(10)),
            );
            collection.and(removal)?;
            std::fs::read(outputs.join("output.bin"))
                .map_err(|_| DockerRuntimeError::new("artifact_gate_output_missing"))
        })();
        let _ = cli.remove_container_without_volumes(
            &workload_name,
            &DockerCommandControl::cleanup(Duration::from_secs(10)),
        );
        let cleanup = cli.cleanup_artifact_storage(
            &plan,
            &DockerCommandControl::cleanup(Duration::from_secs(30)),
        );
        let _ = std::fs::remove_dir_all(&root);
        cleanup.unwrap();
        assert_eq!(result.unwrap(), b"burd:hello burd");
    }

    #[test]
    fn bounded_reader_keeps_only_the_tail() {
        let input = b"0123456789";
        let (output, truncated) = read_tail_bounded(input.as_slice(), 4).unwrap();
        assert_eq!(output, b"6789");
        assert!(truncated);
    }

    #[test]
    #[ignore]
    fn bounded_command_test_helper() {
        if std::env::var_os("BURD_RUNTIME_COMMAND_TEST_HELPER").is_some() {
            thread::sleep(Duration::from_secs(30));
        }
    }

    #[test]
    fn runtime_command_timeout_terminates_the_child() {
        let started_at = Instant::now();
        let error = run_bounded_process(
            &mut sleeping_test_process(),
            &DockerCommandControl::cleanup(Duration::from_millis(100)),
        )
        .err()
        .unwrap();

        assert_eq!(error.code(), "runtime_command_timed_out");
        assert!(started_at.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn runtime_command_cancellation_terminates_the_child() {
        let cancellation = JobCancellation::default();
        let cancellation_request = cancellation.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            cancellation_request.cancel();
        });
        let started_at = Instant::now();
        let error = run_bounded_process(
            &mut sleeping_test_process(),
            &DockerCommandControl::cancellable(Duration::from_secs(5), cancellation),
        )
        .err()
        .unwrap();
        canceller.join().unwrap();

        assert_eq!(error.code(), "runtime_command_cancelled");
        assert!(started_at.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn exact_container_lookup_distinguishes_absence_and_substring_matches() {
        let stdout = "aaaaaaaaaaaa\tburd-job-1-old\nbbbbbbbbbbbb\tburd-job-1\n";
        assert_eq!(
            exact_container_id(stdout, false, "burd-job-1").unwrap(),
            Some("bbbbbbbbbbbb".to_string())
        );
        assert_eq!(exact_container_id("", false, "burd-job-1").unwrap(), None);
    }

    #[test]
    fn exact_container_lookup_rejects_ambiguous_or_truncated_output() {
        let duplicate = "aaaaaaaaaaaa\tburd-job-1\nbbbbbbbbbbbb\tburd-job-1\n";
        assert_eq!(
            exact_container_id(duplicate, false, "burd-job-1")
                .err()
                .unwrap()
                .code(),
            "container_list_invalid"
        );
        assert_eq!(
            exact_container_id("", true, "burd-job-1")
                .err()
                .unwrap()
                .code(),
            "container_list_invalid"
        );
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
