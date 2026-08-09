use crate::docker_runtime_backend::{
    DockerCommandControl, DockerContainerLogs, DockerContainerPlan, DockerContainerState,
    DockerRuntimeBackend, DockerRuntimeError,
};
use crate::provider_job_executor::{
    JobCancellation, ProviderJobAssignment, ProviderJobExecutionError, ProviderJobExecutionOutcome,
    ProviderJobExecutor,
};
use burd_protocol::validate_provider_job_execution_bundle;
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

const MANAGED_LABEL: &str = "com.burd.managed";
const JOB_LABEL: &str = "com.burd.job_id";
const LEASE_LABEL: &str = "com.burd.lease_id";
const PROVIDER_LABEL: &str = "com.burd.provider_id";
const DEVICE_LABEL: &str = "com.burd.device_id";
const SESSION_LABEL: &str = "com.burd.session_id";
const GPU_LABEL: &str = "com.burd.gpu_uuid";
const BACKEND_LABEL: &str = "com.burd.runtime_backend";
const RUNTIME_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const ARTIFACT_BRIDGE_TIMEOUT: Duration = Duration::from_secs(120);
const CLEANUP_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ARTIFACT_WORKSPACE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const MAX_ARTIFACTS: usize = 32;

pub trait ProviderJobImagePolicy: Send + Sync + 'static {
    fn image_is_allowed(&self, template_id: &str, image_ref: &str) -> bool;
}

/// Exact template/digest pairs accepted by this Agent process.
///
/// An empty policy is fail-closed. The executor has no permissive default.
#[derive(Clone, Debug, Default)]
pub struct StaticProviderJobImagePolicy {
    allowed: BTreeSet<(String, String)>,
}

impl StaticProviderJobImagePolicy {
    pub fn new(allowed: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        Self {
            allowed: allowed
                .into_iter()
                .map(|(template_id, image_ref)| (template_id.into(), image_ref.into()))
                .collect(),
        }
    }
}

impl ProviderJobImagePolicy for StaticProviderJobImagePolicy {
    fn image_is_allowed(&self, template_id: &str, image_ref: &str) -> bool {
        self.allowed
            .contains(&(template_id.to_string(), image_ref.to_string()))
    }
}

trait RuntimeClock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
    fn sleep(&self, duration: Duration);
}

struct SystemRuntimeClock;

impl RuntimeClock for SystemRuntimeClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

pub struct DockerNvidiaProviderJobExecutor<B, P> {
    backend: B,
    image_policy: P,
    clock: Arc<dyn RuntimeClock>,
}

impl<B, P> DockerNvidiaProviderJobExecutor<B, P>
where
    B: DockerRuntimeBackend,
    P: ProviderJobImagePolicy,
{
    pub fn new(backend: B, image_policy: P) -> Self {
        Self {
            backend,
            image_policy,
            clock: Arc::new(SystemRuntimeClock),
        }
    }

    #[cfg(test)]
    fn with_clock(backend: B, image_policy: P, clock: Arc<dyn RuntimeClock>) -> Self {
        Self {
            backend,
            image_policy,
            clock,
        }
    }

    fn execute_inner(
        &self,
        assignment: ProviderJobAssignment,
        cancellation: JobCancellation,
    ) -> Result<ProviderJobExecutionOutcome, ProviderJobExecutionError> {
        validate_provider_job_execution_bundle(
            &assignment.job,
            &assignment.lease,
            &assignment.data_plane,
            &assignment.execution,
        )
        .map_err(|_| execution_error("executor_contract_invalid"))?;
        cancellation.ensure_not_cancelled()?;
        validate_runtime_identifiers(&assignment)?;
        validate_workspace_binding(&assignment)?;
        if !self.image_policy.image_is_allowed(
            &assignment.execution.template_id,
            &assignment.execution.image_ref,
        ) {
            return Err(execution_error("container_image_not_allowed"));
        }

        let started_at = self.clock.now();
        let deadlines = execution_deadlines(&assignment, started_at)?;
        let plan = build_container_plan(&assignment, self.backend.runtime_backend());

        // All checks above and all environment probes are read-only. Container side effects start
        // only after the complete contract, image policy, deadline, host, GPU and image pass.
        self.run_active_command(&cancellation, deadlines, |control| {
            self.backend.verify_environment(&plan, control)
        })?;
        self.remove_stale_container(&plan, &cancellation, deadlines)?;
        if plan.artifact_workspace
            && let Err(error) = self.run_active_command(&cancellation, deadlines, |control| {
                self.backend.prepare_artifacts(&plan, control)
            })
        {
            let _ = self
                .backend
                .cleanup_artifacts(&plan, &self.cleanup_command_control());
            return Err(error);
        }

        let container_id = match self.run_active_command(&cancellation, deadlines, |control| {
            self.backend.create(&plan, control)
        }) {
            Ok(container_id) => container_id,
            Err(error) => {
                self.cleanup_partial_create(&plan)?;
                return Err(error);
            }
        };
        if let Some(workspace) = assignment.workspace.as_ref()
            && let Err(error) = self.run_artifact_bridge(&cancellation, deadlines, |control| {
                self.backend
                    .stage_inputs(&plan, &workspace.inputs_dir, control)
            })
        {
            self.remove_created_container(&container_id, &plan)?;
            return Err(error);
        }
        if let Err(error) = self.run_active_command(&cancellation, deadlines, |control| {
            self.backend.start(&container_id, control)
        }) {
            self.remove_created_container(&container_id, &plan)?;
            return Err(error);
        }

        let monitor_result =
            self.monitor_container(&container_id, &assignment, &cancellation, deadlines);
        match monitor_result {
            Ok(state) => self.finish_exited_container(ExitedContainer {
                container_id: &container_id,
                plan: &plan,
                assignment: &assignment,
                cancellation: &cancellation,
                deadlines,
                state,
                started_at,
            }),
            Err(error) => {
                let termination = self.terminate_running_container(&container_id, &assignment);
                let removal = self.remove_created_container(&container_id, &plan);
                if termination.is_err() || removal.is_err() {
                    Err(execution_error("container_cleanup_failed"))
                } else {
                    Err(error)
                }
            }
        }
    }

    fn remove_stale_container(
        &self,
        plan: &DockerContainerPlan,
        cancellation: &JobCancellation,
        deadlines: ExecutionDeadlines,
    ) -> Result<(), ProviderJobExecutionError> {
        let Some(existing) = self.run_active_command(cancellation, deadlines, |control| {
            self.backend.existing_container(&plan.name, control)
        })?
        else {
            return Ok(());
        };
        if !labels_match(&existing.labels, &plan.labels) {
            return Err(execution_error("container_name_conflict"));
        }
        self.run_active_command(cancellation, deadlines, |control| {
            self.backend.remove(&plan.name, control)
        })
    }

    fn cleanup_partial_create(
        &self,
        plan: &DockerContainerPlan,
    ) -> Result<(), ProviderJobExecutionError> {
        let container_cleanup = (|| {
            let existing = self
                .backend
                .existing_container(&plan.name, &self.cleanup_command_control())
                .map_err(map_runtime_error)?;
            if let Some(existing) = existing {
                if !labels_match(&existing.labels, &plan.labels) {
                    return Err(execution_error("container_name_conflict"));
                }
                self.backend
                    .remove(&plan.name, &self.cleanup_command_control())
                    .map_err(map_runtime_error)?;
            }
            Ok(())
        })();
        let artifact_cleanup = self
            .backend
            .cleanup_artifacts(plan, &self.cleanup_command_control())
            .map_err(map_runtime_error);
        container_cleanup.and(artifact_cleanup)
    }

    fn remove_created_container(
        &self,
        container_id: &str,
        plan: &DockerContainerPlan,
    ) -> Result<(), ProviderJobExecutionError> {
        let container_cleanup = self
            .backend
            .remove(container_id, &self.cleanup_command_control())
            .map_err(|_| execution_error("container_cleanup_failed"));
        let artifact_cleanup = self
            .backend
            .cleanup_artifacts(plan, &self.cleanup_command_control())
            .map_err(|_| execution_error("container_cleanup_failed"));
        container_cleanup.and(artifact_cleanup)
    }

    fn monitor_container(
        &self,
        container_id: &str,
        assignment: &ProviderJobAssignment,
        cancellation: &JobCancellation,
        deadlines: ExecutionDeadlines,
    ) -> Result<DockerContainerState, ProviderJobExecutionError> {
        let poll_interval = Duration::from_secs(u64::from(
            assignment.execution.cancellation.poll_interval_seconds,
        ));
        loop {
            let now = self.clock.now();
            if cancellation.requested() {
                return Err(execution_error("execution_cancelled"));
            }
            if now >= deadlines.lease {
                return Err(execution_error("execution_lease_expired"));
            }
            if now >= deadlines.timeout {
                return Err(execution_error("execution_timeout"));
            }
            let state = self.run_active_command(cancellation, deadlines, |control| {
                self.backend.inspect(container_id, control)
            })?;
            if !state.running {
                return Ok(state);
            }
            if cancellation.requested() {
                continue;
            }
            let remaining = (deadlines.lease.min(deadlines.timeout) - self.clock.now())
                .to_std()
                .unwrap_or(Duration::ZERO);
            self.clock.sleep(poll_interval.min(remaining));
        }
    }

    fn terminate_running_container(
        &self,
        container_id: &str,
        assignment: &ProviderJobAssignment,
    ) -> Result<(), DockerRuntimeError> {
        let policy = &assignment.execution.cancellation;
        let started_at = self.clock.now();
        let graceful_deadline = add_seconds(started_at, policy.graceful_stop_seconds);
        let force_deadline = add_seconds(started_at, policy.force_kill_after_seconds);
        let _ = self
            .backend
            .terminate(container_id, &self.cleanup_control_until(force_deadline));
        let poll_interval = Duration::from_secs(u64::from(policy.poll_interval_seconds));

        loop {
            let now = self.clock.now();
            if now >= force_deadline {
                return self
                    .backend
                    .kill(container_id, &self.cleanup_command_control());
            }
            if self
                .backend
                .inspect(container_id, &self.cleanup_control_until(force_deadline))
                .is_ok_and(|state| !state.running)
            {
                return Ok(());
            }
            let phase_deadline = if now < graceful_deadline {
                graceful_deadline
            } else {
                force_deadline
            };
            let remaining = (phase_deadline - self.clock.now())
                .to_std()
                .unwrap_or(Duration::ZERO);
            self.clock.sleep(poll_interval.min(remaining));
        }
    }

    fn finish_exited_container(
        &self,
        exited: ExitedContainer<'_>,
    ) -> Result<ProviderJobExecutionOutcome, ProviderJobExecutionError> {
        let logs = match self.run_active_command(exited.cancellation, exited.deadlines, |control| {
            self.backend.logs(exited.container_id, control)
        }) {
            Ok(logs) => logs,
            Err(error) => {
                self.remove_created_container(exited.container_id, exited.plan)?;
                return Err(error);
            }
        };
        if let Some(workspace) = exited.assignment.workspace.as_ref()
            && let Err(error) =
                self.run_artifact_bridge(exited.cancellation, exited.deadlines, |control| {
                    self.backend
                        .collect_outputs(exited.plan, &workspace.outputs_dir, control)
                })
        {
            self.remove_created_container(exited.container_id, exited.plan)?;
            return Err(error);
        }
        self.remove_created_container(exited.container_id, exited.plan)?;

        if exited.state.oom_killed {
            return Err(execution_error("container_oom_killed"));
        }
        let exit_code = exited
            .state
            .exit_code
            .ok_or_else(|| execution_error("container_exit_code_missing"))?;
        if exit_code != 0 {
            return Err(execution_error("container_exit_nonzero"));
        }

        let finished_at = self.clock.now();
        let runtime_duration_ms = (finished_at - exited.started_at).num_milliseconds().max(0);
        Ok(ProviderJobExecutionOutcome::new(
            Vec::new(),
            execution_metrics(
                self.backend.runtime_backend(),
                exited.assignment,
                &exited.state,
                &logs,
                runtime_duration_ms,
            ),
        ))
    }

    fn run_active_command<T>(
        &self,
        cancellation: &JobCancellation,
        deadlines: ExecutionDeadlines,
        operation: impl FnOnce(&DockerCommandControl) -> Result<T, DockerRuntimeError>,
    ) -> Result<T, ProviderJobExecutionError> {
        let active =
            self.active_command_control(cancellation, deadlines, RUNTIME_COMMAND_TIMEOUT)?;
        operation(&active.control)
            .map_err(|error| map_active_runtime_error(error, active.timeout_code))
    }

    fn run_artifact_bridge<T>(
        &self,
        cancellation: &JobCancellation,
        deadlines: ExecutionDeadlines,
        operation: impl FnOnce(&DockerCommandControl) -> Result<T, DockerRuntimeError>,
    ) -> Result<T, ProviderJobExecutionError> {
        let active =
            self.active_command_control(cancellation, deadlines, ARTIFACT_BRIDGE_TIMEOUT)?;
        operation(&active.control)
            .map_err(|error| map_active_runtime_error(error, active.timeout_code))
    }

    fn active_command_control(
        &self,
        cancellation: &JobCancellation,
        deadlines: ExecutionDeadlines,
        command_timeout: Duration,
    ) -> Result<ActiveDockerCommandControl, ProviderJobExecutionError> {
        cancellation.ensure_not_cancelled()?;
        let now = self.clock.now();
        if now >= deadlines.lease {
            return Err(execution_error("execution_lease_expired"));
        }
        if now >= deadlines.timeout {
            return Err(execution_error("execution_timeout"));
        }
        let (deadline, deadline_code) = if deadlines.lease <= deadlines.timeout {
            (deadlines.lease, "execution_lease_expired")
        } else {
            (deadlines.timeout, "execution_timeout")
        };
        let remaining = (deadline - now).to_std().unwrap_or(Duration::ZERO);
        let (timeout, timeout_code) = if remaining <= command_timeout {
            (remaining, deadline_code)
        } else {
            (command_timeout, "runtime_command_timed_out")
        };
        Ok(ActiveDockerCommandControl {
            control: DockerCommandControl::cancellable(timeout, cancellation.clone()),
            timeout_code,
        })
    }

    fn cleanup_command_control(&self) -> DockerCommandControl {
        DockerCommandControl::cleanup(CLEANUP_COMMAND_TIMEOUT)
    }

    fn cleanup_control_until(&self, deadline: DateTime<Utc>) -> DockerCommandControl {
        let remaining = (deadline - self.clock.now())
            .to_std()
            .unwrap_or(Duration::ZERO);
        DockerCommandControl::cleanup(CLEANUP_COMMAND_TIMEOUT.min(remaining))
    }
}

impl<B, P> ProviderJobExecutor for DockerNvidiaProviderJobExecutor<B, P>
where
    B: DockerRuntimeBackend,
    P: ProviderJobImagePolicy,
{
    fn execute(
        &self,
        assignment: ProviderJobAssignment,
        cancellation: JobCancellation,
    ) -> Result<ProviderJobExecutionOutcome, ProviderJobExecutionError> {
        self.execute_inner(assignment, cancellation)
    }
}

#[derive(Clone, Copy)]
struct ExecutionDeadlines {
    lease: DateTime<Utc>,
    timeout: DateTime<Utc>,
}

struct ExitedContainer<'a> {
    container_id: &'a str,
    plan: &'a DockerContainerPlan,
    assignment: &'a ProviderJobAssignment,
    cancellation: &'a JobCancellation,
    deadlines: ExecutionDeadlines,
    state: DockerContainerState,
    started_at: DateTime<Utc>,
}

struct ActiveDockerCommandControl {
    control: DockerCommandControl,
    timeout_code: &'static str,
}

fn execution_deadlines(
    assignment: &ProviderJobAssignment,
    now: DateTime<Utc>,
) -> Result<ExecutionDeadlines, ProviderJobExecutionError> {
    let lease = DateTime::parse_from_rfc3339(&assignment.execution.lease_expires_at)
        .map_err(|_| execution_error("execution_deadline_invalid"))?
        .with_timezone(&Utc);
    let data_plane =
        DateTime::parse_from_rfc3339(&assignment.execution.data_plane_credential_expires_at)
            .map_err(|_| execution_error("execution_deadline_invalid"))?
            .with_timezone(&Utc);
    if lease <= now || data_plane <= now {
        return Err(execution_error("execution_assignment_expired"));
    }
    let timeout = now
        .checked_add_signed(TimeDelta::seconds(i64::from(
            assignment.execution.timeout_seconds,
        )))
        .ok_or_else(|| execution_error("execution_deadline_invalid"))?;
    Ok(ExecutionDeadlines {
        lease: lease.min(data_plane),
        timeout,
    })
}

fn add_seconds(now: DateTime<Utc>, seconds: u32) -> DateTime<Utc> {
    now.checked_add_signed(TimeDelta::seconds(i64::from(seconds)))
        .unwrap_or(now)
}

fn validate_runtime_identifiers(
    assignment: &ProviderJobAssignment,
) -> Result<(), ProviderJobExecutionError> {
    for value in [
        assignment.job.job_id.as_str(),
        assignment.lease.lease_id.as_str(),
        assignment.job.provider_id.as_str(),
        assignment.job.device_id.as_str(),
        assignment.job.session_id.as_str(),
        assignment.job.gpu_uuid.as_str(),
    ] {
        if value.is_empty()
            || value.len() > 128
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(execution_error("runtime_identifier_invalid"));
        }
    }
    Ok(())
}

fn validate_workspace_binding(
    assignment: &ProviderJobAssignment,
) -> Result<(), ProviderJobExecutionError> {
    let artifacts_declared =
        !assignment.job.input_artifacts.is_empty() || !assignment.job.expected_outputs.is_empty();
    if artifacts_declared != assignment.workspace.is_some() {
        return Err(execution_error("artifact_workspace_mismatch"));
    }
    if assignment.job.input_artifacts.len() > MAX_ARTIFACTS
        || assignment.job.expected_outputs.len() > MAX_ARTIFACTS
    {
        return Err(execution_error("artifact_count_invalid"));
    }
    for (artifacts, code) in [
        (
            assignment.job.input_artifacts.as_slice(),
            "artifact_input_limit_invalid",
        ),
        (
            assignment.job.expected_outputs.as_slice(),
            "artifact_output_limit_invalid",
        ),
    ] {
        artifacts
            .iter()
            .map(|artifact| artifact.size_bytes)
            .try_fold(0_u64, |total, size| total.checked_add(size?))
            .filter(|total| *total <= MAX_ARTIFACT_WORKSPACE_BYTES)
            .ok_or_else(|| execution_error(code))?;
    }
    Ok(())
}

fn build_container_plan(
    assignment: &ProviderJobAssignment,
    runtime_backend: &str,
) -> DockerContainerPlan {
    let mut labels = BTreeMap::new();
    labels.insert(MANAGED_LABEL.to_string(), "true".to_string());
    labels.insert(JOB_LABEL.to_string(), assignment.job.job_id.clone());
    labels.insert(LEASE_LABEL.to_string(), assignment.lease.lease_id.clone());
    labels.insert(
        PROVIDER_LABEL.to_string(),
        assignment.job.provider_id.clone(),
    );
    labels.insert(DEVICE_LABEL.to_string(), assignment.job.device_id.clone());
    labels.insert(SESSION_LABEL.to_string(), assignment.job.session_id.clone());
    labels.insert(GPU_LABEL.to_string(), assignment.job.gpu_uuid.clone());
    labels.insert(BACKEND_LABEL.to_string(), runtime_backend.to_string());
    let input_artifact_bytes = assignment
        .job
        .input_artifacts
        .iter()
        .filter_map(|artifact| artifact.size_bytes)
        .fold(0_u64, u64::saturating_add);
    let output_artifact_bytes = assignment
        .job
        .expected_outputs
        .iter()
        .filter_map(|artifact| artifact.size_bytes)
        .fold(0_u64, u64::saturating_add);
    DockerContainerPlan {
        name: container_name(&assignment.job.job_id, &assignment.lease.lease_id),
        image_ref: assignment.execution.image_ref.clone(),
        gpu_uuid: assignment.execution.gpu_uuid.clone(),
        user: assignment.execution.runtime.run_as_user.clone(),
        cpu_millis: assignment.execution.runtime.cpu_millis,
        memory_mib: assignment.execution.runtime.memory_mib,
        pids_limit: assignment.execution.runtime.pids_limit,
        shm_size_mib: assignment.execution.runtime.shm_size_mib,
        labels,
        environment: BTreeMap::new(),
        artifact_workspace: assignment.workspace.is_some(),
        input_artifact_count: assignment.job.input_artifacts.len() as u32,
        output_artifact_count: assignment.job.expected_outputs.len() as u32,
        input_artifact_bytes,
        output_artifact_bytes,
    }
}

fn container_name(job_id: &str, lease_id: &str) -> String {
    let suffix = job_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(20)
        .collect::<String>();
    let suffix = if suffix.is_empty() { "job" } else { &suffix };
    format!(
        "burd-job-{suffix}-{:016x}",
        stable_assignment_hash(job_id, lease_id)
    )
}

fn stable_assignment_hash(job_id: &str, lease_id: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in job_id.bytes().chain([0]).chain(lease_id.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn labels_match(existing: &BTreeMap<String, String>, expected: &BTreeMap<String, String>) -> bool {
    expected
        .iter()
        .all(|(key, value)| existing.get(key) == Some(value))
}

fn execution_metrics(
    runtime_backend: &str,
    assignment: &ProviderJobAssignment,
    state: &DockerContainerState,
    logs: &DockerContainerLogs,
    runtime_duration_ms: i64,
) -> serde_json::Value {
    let artifact_workload = assignment.workspace.is_some();
    json!({
        "runtime_engine": "docker",
        "runtime_backend": runtime_backend,
        "container_os": assignment.execution.runtime.container_os,
        "gpu_backend": assignment.execution.runtime.gpu_backend,
        "gpu_runtime": assignment.execution.runtime.gpu_runtime,
        "gpu_uuid": assignment.execution.gpu_uuid,
        "exit_code": state.exit_code,
        "oom_killed": state.oom_killed,
        "started_at": state.started_at,
        "finished_at": state.finished_at,
        "runtime_duration_ms": runtime_duration_ms,
        "termination_reason": "completed",
        "cleanup_status": "completed",
        "stdout_tail": if artifact_workload { "" } else { logs.stdout_tail() },
        "stderr_tail": if artifact_workload { "" } else { logs.stderr_tail() },
        "stdout_truncated": logs.stdout_truncated(),
        "stderr_truncated": logs.stderr_truncated(),
        "workload_logs_redacted": artifact_workload,
    })
}

fn execution_error(code: &'static str) -> ProviderJobExecutionError {
    ProviderJobExecutionError::new(code, "Docker NVIDIA provider execution failed")
}

fn map_runtime_error(error: DockerRuntimeError) -> ProviderJobExecutionError {
    execution_error(error.code())
}

fn map_active_runtime_error(
    error: DockerRuntimeError,
    timeout_code: &'static str,
) -> ProviderJobExecutionError {
    match error.code() {
        "runtime_command_cancelled" => execution_error("execution_cancelled"),
        "runtime_command_timed_out" => execution_error(timeout_code),
        _ => map_runtime_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker_runtime_backend::{ExistingDockerContainer, MAX_DOCKER_LOG_BYTES};
    use burd_protocol::{
        JOB_DATA_PLANE_GRANT_VERSION, JOB_LEASE_SCHEMA_VERSION, JOB_SCHEMA_VERSION, JobArtifact,
        JobDataPlaneGrant, JobDataPlaneUrl, JobLeaseRecord, JobRecord,
        PROVIDER_JOB_EXECUTION_POLICY_VERSION, PROVIDER_JOB_EXECUTION_SCHEMA_VERSION,
        ProviderJobCancellationPolicy, ProviderJobCleanupPolicy, ProviderJobExecutionSpec,
        ProviderJobExecutionState, ProviderJobRuntimePolicy,
    };
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct FakeClock {
        now: Arc<Mutex<DateTime<Utc>>>,
    }

    impl FakeClock {
        fn new(now: DateTime<Utc>) -> Self {
            Self {
                now: Arc::new(Mutex::new(now)),
            }
        }
    }

    impl RuntimeClock for FakeClock {
        fn now(&self) -> DateTime<Utc> {
            *self.now.lock().unwrap()
        }

        fn sleep(&self, duration: Duration) {
            let delta = TimeDelta::from_std(duration).unwrap();
            let mut now = self.now.lock().unwrap();
            *now += delta;
        }
    }

    #[derive(Default)]
    struct FakeBackendState {
        operations: Vec<String>,
        existing: VecDeque<Option<ExistingDockerContainer>>,
        inspect: VecDeque<DockerContainerState>,
        logs: DockerContainerLogs,
        fail_existing: bool,
        fail_create: bool,
        fail_start: bool,
        fail_terminate: bool,
        fail_kill: bool,
        fail_remove: bool,
        cancel_on_inspect: Option<JobCancellation>,
        observed_plans: Vec<DockerContainerPlan>,
    }

    #[derive(Clone, Default)]
    struct FakeBackend {
        state: Arc<Mutex<FakeBackendState>>,
    }

    impl FakeBackend {
        fn operations(&self) -> Vec<String> {
            self.state.lock().unwrap().operations.clone()
        }

        fn plan(&self) -> DockerContainerPlan {
            self.state.lock().unwrap().observed_plans[0].clone()
        }
    }

    impl DockerRuntimeBackend for FakeBackend {
        fn runtime_backend(&self) -> &'static str {
            "docker_linux_native"
        }

        fn verify_platform(
            &self,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            Ok(())
        }

        fn verify_environment(
            &self,
            plan: &DockerContainerPlan,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            let mut state = self.state.lock().unwrap();
            state.operations.push("verify".to_string());
            state.observed_plans.push(plan.clone());
            Ok(())
        }

        fn runtime_environment(
            &self,
            _control: &DockerCommandControl,
        ) -> Result<crate::docker_runtime_backend::DockerRuntimeEnvironment, DockerRuntimeError>
        {
            Ok(crate::docker_runtime_backend::DockerRuntimeEnvironment {
                docker_server_version: "test-docker".to_string(),
                nvidia_driver_version: "test-driver".to_string(),
                nvidia_runtime: "nvidia".to_string(),
                gpu_uuids: vec!["GPU-test".to_string()],
            })
        }

        fn existing_container(
            &self,
            _name: &str,
            _control: &DockerCommandControl,
        ) -> Result<Option<ExistingDockerContainer>, DockerRuntimeError> {
            let mut state = self.state.lock().unwrap();
            state.operations.push("existing".to_string());
            if state.fail_existing {
                Err(DockerRuntimeError::new("container_inspect_failed"))
            } else {
                Ok(state.existing.pop_front().flatten())
            }
        }

        fn create(
            &self,
            _plan: &DockerContainerPlan,
            _control: &DockerCommandControl,
        ) -> Result<String, DockerRuntimeError> {
            let mut state = self.state.lock().unwrap();
            state.operations.push("create".to_string());
            if state.fail_create {
                Err(DockerRuntimeError::new("container_create_failed"))
            } else {
                Ok("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string())
            }
        }

        fn start(
            &self,
            _container_id: &str,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            let mut state = self.state.lock().unwrap();
            state.operations.push("start".to_string());
            if state.fail_start {
                Err(DockerRuntimeError::new("container_start_failed"))
            } else {
                Ok(())
            }
        }

        fn prepare_artifacts(
            &self,
            plan: &DockerContainerPlan,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            if plan.artifact_workspace {
                self.state
                    .lock()
                    .unwrap()
                    .operations
                    .push("prepare_artifacts".to_string());
            }
            Ok(())
        }

        fn stage_inputs(
            &self,
            _plan: &DockerContainerPlan,
            _inputs_dir: &std::path::Path,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            self.state
                .lock()
                .unwrap()
                .operations
                .push("stage_inputs".to_string());
            Ok(())
        }

        fn collect_outputs(
            &self,
            _plan: &DockerContainerPlan,
            _outputs_dir: &std::path::Path,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            self.state
                .lock()
                .unwrap()
                .operations
                .push("collect_outputs".to_string());
            Ok(())
        }

        fn cleanup_artifacts(
            &self,
            plan: &DockerContainerPlan,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            if plan.artifact_workspace {
                self.state
                    .lock()
                    .unwrap()
                    .operations
                    .push("cleanup_artifacts".to_string());
            }
            Ok(())
        }

        fn inspect(
            &self,
            _container_id: &str,
            _control: &DockerCommandControl,
        ) -> Result<DockerContainerState, DockerRuntimeError> {
            let mut state = self.state.lock().unwrap();
            state.operations.push("inspect".to_string());
            if let Some(cancellation) = state.cancel_on_inspect.take() {
                cancellation.cancel();
            }
            Ok(state.inspect.pop_front().unwrap_or_else(running_state))
        }

        fn logs(
            &self,
            _container_id: &str,
            _control: &DockerCommandControl,
        ) -> Result<DockerContainerLogs, DockerRuntimeError> {
            let mut state = self.state.lock().unwrap();
            state.operations.push("logs".to_string());
            Ok(state.logs.clone())
        }

        fn terminate(
            &self,
            _container_id: &str,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            let mut state = self.state.lock().unwrap();
            state.operations.push("terminate".to_string());
            if state.fail_terminate {
                Err(DockerRuntimeError::new("container_terminate_failed"))
            } else {
                Ok(())
            }
        }

        fn kill(
            &self,
            _container_id: &str,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            let mut state = self.state.lock().unwrap();
            state.operations.push("kill".to_string());
            if state.fail_kill {
                Err(DockerRuntimeError::new("container_kill_failed"))
            } else {
                Ok(())
            }
        }

        fn remove(
            &self,
            _container_id_or_name: &str,
            _control: &DockerCommandControl,
        ) -> Result<(), DockerRuntimeError> {
            let mut state = self.state.lock().unwrap();
            state.operations.push("remove".to_string());
            if state.fail_remove {
                Err(DockerRuntimeError::new("container_remove_failed"))
            } else {
                Ok(())
            }
        }
    }

    fn running_state() -> DockerContainerState {
        DockerContainerState {
            running: true,
            exit_code: None,
            oom_killed: false,
            started_at: Some("2026-08-07T00:00:00Z".to_string()),
            finished_at: None,
        }
    }

    fn exited_state(exit_code: i32, oom_killed: bool) -> DockerContainerState {
        DockerContainerState {
            running: false,
            exit_code: Some(exit_code),
            oom_killed,
            started_at: Some("2026-08-07T00:00:00Z".to_string()),
            finished_at: Some("2026-08-07T00:00:01Z".to_string()),
        }
    }

    fn assignment_at(now: DateTime<Utc>) -> ProviderJobAssignment {
        let image_ref = format!("ghcr.io/burd/runtime/llm@sha256:{}", "a".repeat(64));
        let lease_expires_at = (now + TimeDelta::minutes(5)).to_rfc3339();
        let credential_expires_at = (now + TimeDelta::minutes(20)).to_rfc3339();
        let job = JobRecord {
            job_id: "job_1".to_string(),
            client_job_id: None,
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            schema_version: JOB_SCHEMA_VERSION.to_string(),
            workload_type: "llm_realtime_api".to_string(),
            template_id: "llm_inference".to_string(),
            image_ref: image_ref.clone(),
            gpu_uuid: "GPU-test".to_string(),
            backend: "cuda".to_string(),
            parameters: json!({}),
            input_artifacts: Vec::new(),
            expected_outputs: Vec::new(),
            result_artifacts: Vec::new(),
            policy_id: Some("policy_1".to_string()),
            policy_version: Some("v1".to_string()),
            status: "assigned".to_string(),
            progress_percent: None,
            status_message: None,
            error_code: None,
            error_message: None,
            cancellation_reason: None,
            timeout_seconds: 30,
            created_at: now.to_rfc3339(),
            assigned_at: Some(now.to_rfc3339()),
            accepted_at: None,
            started_at: None,
            completed_at: None,
            updated_at: now.to_rfc3339(),
        };
        let lease = JobLeaseRecord {
            lease_id: "lease_1".to_string(),
            job_id: job.job_id.clone(),
            provider_id: job.provider_id.clone(),
            device_id: job.device_id.clone(),
            session_id: job.session_id.clone(),
            schema_version: JOB_LEASE_SCHEMA_VERSION.to_string(),
            workload_type: job.workload_type.clone(),
            gpu_uuid: job.gpu_uuid.clone(),
            policy_id: job.policy_id.clone(),
            policy_version: job.policy_version.clone(),
            status: "offered".to_string(),
            reason_codes: Vec::new(),
            offered_at: now.to_rfc3339(),
            expires_at: lease_expires_at.clone(),
            accepted_at: None,
            provisioning_at: None,
            active_at: None,
            completed_at: None,
            failure_reason: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
        };
        let data_plane = JobDataPlaneGrant {
            schema_version: JOB_DATA_PLANE_GRANT_VERSION.to_string(),
            job_id: job.job_id.clone(),
            credential: "jobcred_must_not_reach_backend".to_string(),
            credential_expires_at: credential_expires_at.clone(),
            download_urls: Vec::new(),
            upload_urls: Vec::new(),
        };
        let execution = ProviderJobExecutionSpec {
            schema_version: PROVIDER_JOB_EXECUTION_SCHEMA_VERSION.to_string(),
            policy_version: PROVIDER_JOB_EXECUTION_POLICY_VERSION.to_string(),
            job_schema_version: job.schema_version.clone(),
            lease_schema_version: lease.schema_version.clone(),
            data_plane_schema_version: data_plane.schema_version.clone(),
            job_id: job.job_id.clone(),
            lease_id: lease.lease_id.clone(),
            provider_id: job.provider_id.clone(),
            device_id: job.device_id.clone(),
            session_id: job.session_id.clone(),
            workload_type: job.workload_type.clone(),
            template_id: job.template_id.clone(),
            image_ref: job.image_ref.clone(),
            gpu_uuid: job.gpu_uuid.clone(),
            backend: job.backend.clone(),
            policy_id: job.policy_id.clone(),
            workload_policy_version: job.policy_version.clone(),
            initial_state: ProviderJobExecutionState::Assigned,
            timeout_seconds: job.timeout_seconds,
            lease_expires_at,
            data_plane_credential_expires_at: credential_expires_at,
            runtime: ProviderJobRuntimePolicy::v2(),
            cancellation: ProviderJobCancellationPolicy {
                poll_interval_seconds: 1,
                max_control_silence_seconds: 2,
                graceful_stop_seconds: 1,
                force_kill_after_seconds: 2,
            },
            cleanup: ProviderJobCleanupPolicy::v1(),
        };
        ProviderJobAssignment {
            job,
            lease,
            data_plane,
            execution,
            workspace: None,
        }
    }

    fn executor(
        backend: FakeBackend,
        now: DateTime<Utc>,
        assignment: &ProviderJobAssignment,
    ) -> DockerNvidiaProviderJobExecutor<FakeBackend, StaticProviderJobImagePolicy> {
        DockerNvidiaProviderJobExecutor::with_clock(
            backend,
            StaticProviderJobImagePolicy::new([(
                assignment.execution.template_id.clone(),
                assignment.execution.image_ref.clone(),
            )]),
            Arc::new(FakeClock::new(now)),
        )
    }

    #[test]
    fn successful_execution_is_hardened_bounded_and_cleaned() {
        let now = DateTime::parse_from_rfc3339("2026-08-07T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let assignment = assignment_at(now);
        let backend = FakeBackend::default();
        {
            let mut state = backend.state.lock().unwrap();
            state.inspect.push_back(exited_state(0, false));
            state.logs = DockerContainerLogs::new("completed", "", false, false);
        }
        let outcome = executor(backend.clone(), now, &assignment)
            .execute(assignment, JobCancellation::default())
            .unwrap();

        assert_eq!(
            backend.operations(),
            [
                "verify", "existing", "create", "start", "inspect", "logs", "remove"
            ]
        );
        let plan = backend.plan();
        assert_eq!(plan.gpu_uuid, "GPU-test");
        assert_eq!(
            plan.labels.get(MANAGED_LABEL).map(String::as_str),
            Some("true")
        );
        assert_eq!(outcome.metrics["exit_code"], 0);
        assert_eq!(outcome.metrics["cleanup_status"], "completed");
        assert_eq!(outcome.metrics["stdout_tail"], "completed");
        assert!(!format!("{plan:?}").contains("jobcred_must_not_reach_backend"));
    }

    #[test]
    fn artifact_workspace_uses_runtime_bridge_without_entering_the_plan() {
        let now = Utc::now();
        let root = std::env::temp_dir().join("burd-executor-workspace-test");
        let mut assignment = assignment_at(now);
        assignment.job.input_artifacts = vec![JobArtifact {
            artifact_id: "input".to_string(),
            role: "input".to_string(),
            object_key: "jobs/job_1/input".to_string(),
            sha256: Some(format!("sha256:{}", "a".repeat(64))),
            size_bytes: Some(1),
            content_type: Some("application/octet-stream".to_string()),
        }];
        assignment.job.expected_outputs = vec![JobArtifact {
            artifact_id: "output".to_string(),
            role: "output".to_string(),
            object_key: "jobs/job_1/output".to_string(),
            sha256: None,
            size_bytes: Some(1024),
            content_type: Some("application/octet-stream".to_string()),
        }];
        assignment.data_plane.download_urls = vec![JobDataPlaneUrl {
            artifact_id: "input".to_string(),
            method: "GET".to_string(),
            url: "/v1/jobs/job_1/artifacts/input/download".to_string(),
            expires_at: assignment.data_plane.credential_expires_at.clone(),
        }];
        assignment.data_plane.upload_urls = vec![JobDataPlaneUrl {
            artifact_id: "output".to_string(),
            method: "PUT".to_string(),
            url: "/v1/jobs/job_1/results/output/upload".to_string(),
            expires_at: assignment.data_plane.credential_expires_at.clone(),
        }];
        assignment.workspace = Some(
            crate::provider_job_executor::ProviderJobExecutionWorkspace {
                inputs_dir: root.join("inputs"),
                outputs_dir: root.join("outputs"),
                root,
            },
        );
        let backend = FakeBackend::default();
        backend
            .state
            .lock()
            .unwrap()
            .inspect
            .push_back(exited_state(0, false));

        let outcome = executor(backend.clone(), now, &assignment)
            .execute(assignment, JobCancellation::default())
            .unwrap();

        assert_eq!(
            backend.operations(),
            [
                "verify",
                "existing",
                "prepare_artifacts",
                "create",
                "stage_inputs",
                "start",
                "inspect",
                "logs",
                "collect_outputs",
                "remove",
                "cleanup_artifacts",
            ]
        );
        let plan = backend.plan();
        assert!(plan.artifact_workspace);
        assert_eq!(plan.input_artifact_bytes, 1);
        assert_eq!(plan.output_artifact_bytes, 1024);
        assert!(!format!("{plan:?}").contains("burd-executor-workspace-test"));
        assert_eq!(outcome.metrics["stdout_tail"], "");
        assert_eq!(outcome.metrics["workload_logs_redacted"], true);
    }

    #[test]
    fn invalid_bundle_and_unapproved_image_have_no_side_effects() {
        let now = Utc::now();
        let mut invalid = assignment_at(now);
        invalid.execution.runtime.network_mode = "host".to_string();
        let backend = FakeBackend::default();
        let error = executor(backend.clone(), now, &invalid)
            .execute(invalid, JobCancellation::default())
            .err()
            .unwrap();
        assert_eq!(error.code(), "executor_contract_invalid");
        assert!(backend.operations().is_empty());

        let assignment = assignment_at(now);
        let executor = DockerNvidiaProviderJobExecutor::with_clock(
            backend.clone(),
            StaticProviderJobImagePolicy::default(),
            Arc::new(FakeClock::new(now)),
        );
        let error = executor
            .execute(assignment, JobCancellation::default())
            .err()
            .unwrap();
        assert_eq!(error.code(), "container_image_not_allowed");
        assert!(backend.operations().is_empty());
    }

    #[test]
    fn stale_managed_container_is_removed_but_foreign_conflict_is_not() {
        let now = Utc::now();
        let assignment = assignment_at(now);
        let expected_plan = build_container_plan(&assignment, "docker_linux_native");
        let backend = FakeBackend::default();
        {
            let mut state = backend.state.lock().unwrap();
            state.existing.push_back(Some(ExistingDockerContainer {
                labels: expected_plan.labels.clone(),
            }));
            state.inspect.push_back(exited_state(0, false));
        }
        executor(backend.clone(), now, &assignment)
            .execute(assignment.clone(), JobCancellation::default())
            .unwrap();
        assert_eq!(
            backend.operations(),
            [
                "verify", "existing", "remove", "create", "start", "inspect", "logs", "remove"
            ]
        );

        let foreign = FakeBackend::default();
        foreign
            .state
            .lock()
            .unwrap()
            .existing
            .push_back(Some(ExistingDockerContainer {
                labels: BTreeMap::from([(MANAGED_LABEL.to_string(), "false".to_string())]),
            }));
        let error = executor(foreign.clone(), now, &assignment)
            .execute(assignment, JobCancellation::default())
            .err()
            .unwrap();
        assert_eq!(error.code(), "container_name_conflict");
        assert_eq!(foreign.operations(), ["verify", "existing"]);
    }

    #[test]
    fn partial_create_and_start_failures_are_cleaned() {
        let now = Utc::now();
        let assignment = assignment_at(now);
        let plan = build_container_plan(&assignment, "docker_linux_native");
        let backend = FakeBackend::default();
        {
            let mut state = backend.state.lock().unwrap();
            state.fail_create = true;
            state.existing.push_back(None);
            state.existing.push_back(Some(ExistingDockerContainer {
                labels: plan.labels,
            }));
        }
        let error = executor(backend.clone(), now, &assignment)
            .execute(assignment.clone(), JobCancellation::default())
            .err()
            .unwrap();
        assert_eq!(error.code(), "container_create_failed");
        assert_eq!(
            backend.operations(),
            ["verify", "existing", "create", "existing", "remove"]
        );

        let start_failure = FakeBackend::default();
        start_failure.state.lock().unwrap().fail_start = true;
        let error = executor(start_failure.clone(), now, &assignment)
            .execute(assignment, JobCancellation::default())
            .err()
            .unwrap();
        assert_eq!(error.code(), "container_start_failed");
        assert_eq!(
            start_failure.operations(),
            ["verify", "existing", "create", "start", "remove"]
        );
    }

    #[test]
    fn nonzero_and_oom_exits_are_distinct_and_cleaned() {
        let now = Utc::now();
        for (state, expected) in [
            (exited_state(23, false), "container_exit_nonzero"),
            (exited_state(137, true), "container_oom_killed"),
        ] {
            let assignment = assignment_at(now);
            let backend = FakeBackend::default();
            backend.state.lock().unwrap().inspect.push_back(state);
            let error = executor(backend.clone(), now, &assignment)
                .execute(assignment, JobCancellation::default())
                .err()
                .unwrap();
            assert_eq!(error.code(), expected);
            assert!(
                backend
                    .operations()
                    .ends_with(&["logs".to_string(), "remove".to_string()])
            );
        }
    }

    #[test]
    fn cancellation_terminates_then_forces_kill_at_policy_deadline() {
        let now = Utc::now();
        let assignment = assignment_at(now);
        let cancellation = JobCancellation::default();
        let backend = FakeBackend::default();
        {
            let mut state = backend.state.lock().unwrap();
            state.cancel_on_inspect = Some(cancellation.clone());
            state.fail_terminate = true;
        }
        let error = executor(backend.clone(), now, &assignment)
            .execute(assignment, cancellation)
            .err()
            .unwrap();
        assert_eq!(error.code(), "execution_cancelled");
        assert!(backend.operations().ends_with(&[
            "inspect".to_string(),
            "terminate".to_string(),
            "inspect".to_string(),
            "inspect".to_string(),
            "kill".to_string(),
            "remove".to_string()
        ]));
    }

    #[test]
    fn timeout_is_backend_authoritative_and_cleans_up() {
        let now = Utc::now();
        let mut assignment = assignment_at(now);
        assignment.job.timeout_seconds = 1;
        assignment.execution.timeout_seconds = 1;
        let backend = FakeBackend::default();
        backend
            .state
            .lock()
            .unwrap()
            .inspect
            .push_back(running_state());
        backend
            .state
            .lock()
            .unwrap()
            .inspect
            .push_back(exited_state(143, false));
        let error = executor(backend.clone(), now, &assignment)
            .execute(assignment, JobCancellation::default())
            .err()
            .unwrap();
        assert_eq!(error.code(), "execution_timeout");
        assert!(backend.operations().ends_with(&[
            "terminate".to_string(),
            "inspect".to_string(),
            "remove".to_string()
        ]));
    }

    #[test]
    fn graceful_termination_exits_before_force_kill_deadline() {
        let now = Utc::now();
        let assignment = assignment_at(now);
        let cancellation = JobCancellation::default();
        let backend = FakeBackend::default();
        {
            let mut state = backend.state.lock().unwrap();
            state.cancel_on_inspect = Some(cancellation.clone());
            state.inspect.push_back(running_state());
            state.inspect.push_back(exited_state(143, false));
        }

        let error = executor(backend.clone(), now, &assignment)
            .execute(assignment, cancellation)
            .err()
            .unwrap();

        assert_eq!(error.code(), "execution_cancelled");
        assert!(backend.operations().ends_with(&[
            "inspect".to_string(),
            "terminate".to_string(),
            "inspect".to_string(),
            "remove".to_string()
        ]));
        assert!(!backend.operations().contains(&"kill".to_string()));
    }

    #[test]
    fn existing_container_probe_failure_is_fail_closed() {
        let now = Utc::now();
        let assignment = assignment_at(now);
        let backend = FakeBackend::default();
        backend.state.lock().unwrap().fail_existing = true;

        let error = executor(backend.clone(), now, &assignment)
            .execute(assignment, JobCancellation::default())
            .err()
            .unwrap();

        assert_eq!(error.code(), "container_inspect_failed");
        assert_eq!(backend.operations(), ["verify", "existing"]);
    }

    #[test]
    fn plan_name_is_normalized_and_does_not_expose_credentials() {
        let assignment = assignment_at(Utc::now());
        let plan = build_container_plan(&assignment, "docker_linux_native");
        assert!(plan.name.starts_with("burd-job-job1-"));
        assert!(plan.name.len() < 64);
        let serialized = format!("{plan:?}");
        assert!(!serialized.contains(&assignment.data_plane.credential));
        assert!(!serialized.contains("docker.sock"));
    }

    #[test]
    fn log_contract_has_explicit_limits() {
        assert_eq!(MAX_DOCKER_LOG_BYTES, 65_536);
        assert_eq!(crate::docker_runtime_backend::MAX_DOCKER_LOG_LINES, 200);
        assert_eq!(ARTIFACT_BRIDGE_TIMEOUT, Duration::from_secs(120));
    }

    #[test]
    #[ignore = "requires Linux, Docker, NVIDIA runtime, and a digest-pinned image whose default command prints nvidia-smi -L"]
    fn physical_linux_nvidia_container_sees_only_leased_gpu() {
        use crate::docker_runtime_backend::LinuxNativeDockerBackend;

        let image_ref = std::env::var("BURD_LINUX_NVIDIA_TEST_IMAGE")
            .expect("BURD_LINUX_NVIDIA_TEST_IMAGE is required");
        let gpu_uuid = std::env::var("BURD_LINUX_NVIDIA_TEST_GPU_UUID")
            .expect("BURD_LINUX_NVIDIA_TEST_GPU_UUID is required");
        let now = Utc::now();
        let mut assignment = assignment_at(now);
        assignment.job.image_ref = image_ref.clone();
        assignment.execution.image_ref = image_ref.clone();
        assignment.job.gpu_uuid = gpu_uuid.clone();
        assignment.lease.gpu_uuid = gpu_uuid.clone();
        assignment.execution.gpu_uuid = gpu_uuid.clone();
        let executor = DockerNvidiaProviderJobExecutor::new(
            LinuxNativeDockerBackend::default(),
            StaticProviderJobImagePolicy::new([("llm_inference", image_ref)]),
        );
        let outcome = executor
            .execute(assignment, JobCancellation::default())
            .unwrap();
        let logs = format!(
            "{}\n{}",
            outcome.metrics["stdout_tail"].as_str().unwrap_or_default(),
            outcome.metrics["stderr_tail"].as_str().unwrap_or_default()
        );
        assert!(logs.contains(&gpu_uuid));
        assert_eq!(logs.matches("GPU-").count(), 1);
    }

    #[test]
    #[ignore = "requires Windows, WSL2, a Linux Docker engine, NVIDIA GPU-PV, and a digest-pinned image whose default command prints nvidia-smi -L"]
    fn physical_windows_wsl2_nvidia_container_sees_only_leased_gpu() {
        use crate::docker_runtime_backend::WindowsWsl2DockerBackend;

        let image_ref = std::env::var("BURD_WINDOWS_WSL2_NVIDIA_TEST_IMAGE")
            .expect("BURD_WINDOWS_WSL2_NVIDIA_TEST_IMAGE is required");
        let gpu_uuid = std::env::var("BURD_WINDOWS_WSL2_NVIDIA_TEST_GPU_UUID")
            .expect("BURD_WINDOWS_WSL2_NVIDIA_TEST_GPU_UUID is required");
        let now = Utc::now();
        let mut assignment = assignment_at(now);
        assignment.job.image_ref = image_ref.clone();
        assignment.execution.image_ref = image_ref.clone();
        assignment.job.gpu_uuid = gpu_uuid.clone();
        assignment.lease.gpu_uuid = gpu_uuid.clone();
        assignment.execution.gpu_uuid = gpu_uuid.clone();
        let executor = DockerNvidiaProviderJobExecutor::new(
            WindowsWsl2DockerBackend::default(),
            StaticProviderJobImagePolicy::new([("llm_inference", image_ref)]),
        );
        let outcome = executor
            .execute(assignment, JobCancellation::default())
            .unwrap();
        let logs = format!(
            "{}\n{}",
            outcome.metrics["stdout_tail"].as_str().unwrap_or_default(),
            outcome.metrics["stderr_tail"].as_str().unwrap_or_default()
        );
        assert!(logs.contains(&gpu_uuid));
        assert_eq!(logs.matches("GPU-").count(), 1);
    }
}
