use crate::provider_job_data_plane::{NoArtifactProviderJobDataPlane, ProviderJobDataPlane};
use crate::provider_job_executor::{
    JobCancellation, ProviderJobAssignment, ProviderJobExecutionError, ProviderJobExecutionOutcome,
    ProviderJobExecutor,
};
use crate::remote_enrollment::join_url;
use burd_hardware::{NvidiaTelemetryCollection, collect_nvidia_telemetry};
use burd_protocol::{
    AcceptJobRequest, JobEventRequest, JobEventResponse, JobResponse, NextJobResponse,
    RemoteEnrollmentState, RemoteSessionState, SubmitJobResultRequest, SubmitJobResultResponse,
    load_remote_enrollment, load_remote_session, validate_next_job_execution_response,
    validate_provider_job_execution_bundle,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::fmt::{self, Display, Formatter};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

const JOB_POLL_INTERVAL: Duration = Duration::from_secs(5);
const JOB_HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const JOB_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const COMPLETED_ASSIGNMENT_MEMORY: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderJobWorkerContext {
    pub provider_id: String,
    pub device_id: String,
    pub session_id: String,
    pub gpu_uuids: Vec<String>,
}

pub struct ProviderJobPoll {
    pub context: ProviderJobWorkerContext,
    pub response: NextJobResponse,
}

pub struct ProviderJobControlError {
    kind: &'static str,
    detail: String,
}

impl ProviderJobControlError {
    pub fn new(kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind: safe_control_failure_kind(kind),
            detail: detail.into(),
        }
    }

    pub fn kind(&self) -> &'static str {
        self.kind
    }

    pub fn diagnostic_detail(&self) -> &str {
        &self.detail
    }
}

fn safe_control_failure_kind(kind: &'static str) -> &'static str {
    match kind {
        "local_state" | "session_changed" | "gpu_inventory" | "transport" | "contract"
        | "unauthorized" | "revoked" | "expired" | "conflict" | "not_found" | "server_error"
        | "rejected" => kind,
        _ => "control_plane_failure",
    }
}

impl fmt::Debug for ProviderJobControlError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderJobControlError")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl Display for ProviderJobControlError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "provider job control-plane operation failed ({})",
            self.kind
        )
    }
}

impl std::error::Error for ProviderJobControlError {}

pub trait ProviderJobControlPlane: Send + Sync + 'static {
    fn next_job(&self) -> Result<ProviderJobPoll, ProviderJobControlError>;

    fn accept_job(
        &self,
        context: &ProviderJobWorkerContext,
        job_id: &str,
        request: &AcceptJobRequest,
    ) -> Result<JobResponse, ProviderJobControlError>;

    fn record_event(
        &self,
        context: &ProviderJobWorkerContext,
        job_id: &str,
        request: &JobEventRequest,
    ) -> Result<JobEventResponse, ProviderJobControlError>;

    fn submit_result(
        &self,
        context: &ProviderJobWorkerContext,
        job_id: &str,
        request: &SubmitJobResultRequest,
    ) -> Result<SubmitJobResultResponse, ProviderJobControlError>;
}

pub struct LocalProviderJobControlPlane {
    telemetry_collector: fn(u64) -> Result<NvidiaTelemetryCollection, String>,
}

impl Default for LocalProviderJobControlPlane {
    fn default() -> Self {
        Self {
            telemetry_collector: collect_nvidia_telemetry,
        }
    }
}

impl LocalProviderJobControlPlane {
    #[cfg(any(test, feature = "integration-test-support"))]
    pub fn with_telemetry_collector(
        telemetry_collector: fn(u64) -> Result<NvidiaTelemetryCollection, String>,
    ) -> Self {
        Self {
            telemetry_collector,
        }
    }

    fn load_auth_state(
        &self,
    ) -> Result<(RemoteEnrollmentState, RemoteSessionState), ProviderJobControlError> {
        let enrollment = load_remote_enrollment()
            .map_err(|error| ProviderJobControlError::new("local_state", error))?;
        let session = load_remote_session()
            .map_err(|error| ProviderJobControlError::new("local_state", error))?;
        if enrollment.control_plane_url.trim_end_matches('/')
            != session.control_plane_url.trim_end_matches('/')
        {
            return Err(ProviderJobControlError::new(
                "local_state",
                "remote enrollment and session belong to different Control Planes",
            ));
        }
        Ok((enrollment, session))
    }

    fn load_bound_auth_state(
        &self,
        context: &ProviderJobWorkerContext,
    ) -> Result<(RemoteEnrollmentState, RemoteSessionState), ProviderJobControlError> {
        let (enrollment, session) = self.load_auth_state()?;
        if enrollment.provider_id != context.provider_id
            || enrollment.device_id != context.device_id
            || session.session_id != context.session_id
        {
            return Err(ProviderJobControlError::new(
                "session_changed",
                "remote session changed during provider job execution",
            ));
        }
        Ok((enrollment, session))
    }

    fn context(
        &self,
        enrollment: &RemoteEnrollmentState,
        session: &RemoteSessionState,
    ) -> Result<ProviderJobWorkerContext, ProviderJobControlError> {
        let collection = (self.telemetry_collector)(1)
            .map_err(|error| ProviderJobControlError::new("gpu_inventory", error))?;
        let gpu_uuids = collection
            .samples
            .into_iter()
            .map(|sample| sample.gpu_uuid)
            .collect::<Vec<_>>();
        if gpu_uuids.is_empty() {
            return Err(ProviderJobControlError::new(
                "gpu_inventory",
                "local NVIDIA inventory is empty",
            ));
        }
        Ok(ProviderJobWorkerContext {
            provider_id: enrollment.provider_id.clone(),
            device_id: enrollment.device_id.clone(),
            session_id: session.session_id.clone(),
            gpu_uuids,
        })
    }
}

impl ProviderJobControlPlane for LocalProviderJobControlPlane {
    fn next_job(&self) -> Result<ProviderJobPoll, ProviderJobControlError> {
        let (enrollment, session) = self.load_auth_state()?;
        // Verify the local NVIDIA identity before the backend assignment endpoint performs
        // its authoritative queued -> assigned transition.
        let context = self.context(&enrollment, &session)?;
        let url = join_url(
            &session.control_plane_url,
            &format!("/v1/sessions/{}/jobs/next", session.session_id),
        );
        let response: NextJobResponse = get_json(&url, &enrollment, &session)?;
        Ok(ProviderJobPoll { context, response })
    }

    fn accept_job(
        &self,
        context: &ProviderJobWorkerContext,
        job_id: &str,
        request: &AcceptJobRequest,
    ) -> Result<JobResponse, ProviderJobControlError> {
        let (enrollment, session) = self.load_bound_auth_state(context)?;
        let url = job_session_url(&session, job_id, "accept");
        post_json(&url, &enrollment, &session, request)
    }

    fn record_event(
        &self,
        context: &ProviderJobWorkerContext,
        job_id: &str,
        request: &JobEventRequest,
    ) -> Result<JobEventResponse, ProviderJobControlError> {
        let (enrollment, session) = self.load_bound_auth_state(context)?;
        let url = job_session_url(&session, job_id, "events");
        post_json(&url, &enrollment, &session, request)
    }

    fn submit_result(
        &self,
        context: &ProviderJobWorkerContext,
        job_id: &str,
        request: &SubmitJobResultRequest,
    ) -> Result<SubmitJobResultResponse, ProviderJobControlError> {
        let (enrollment, session) = self.load_bound_auth_state(context)?;
        let url = job_session_url(&session, job_id, "result");
        post_json(&url, &enrollment, &session, request)
    }
}

fn job_session_url(session: &RemoteSessionState, job_id: &str, action: &str) -> String {
    join_url(
        &session.control_plane_url,
        &format!("/v1/sessions/{}/jobs/{job_id}/{action}", session.session_id),
    )
}

fn get_json<T: DeserializeOwned>(
    url: &str,
    enrollment: &RemoteEnrollmentState,
    session: &RemoteSessionState,
) -> Result<T, ProviderJobControlError> {
    let request = ureq::get(url)
        .config()
        .timeout_global(Some(JOB_HTTP_TIMEOUT))
        .http_status_as_error(false)
        .build();
    let mut response = request
        .header(
            "Authorization",
            &format!("Bearer {}", enrollment.credential),
        )
        .header("X-Burd-Session-Token", &session.resume_token)
        .header("X-Burd-Device-Id", &enrollment.device_id)
        .call()
        .map_err(|error| ProviderJobControlError::new("transport", error.to_string()))?;
    parse_response(&mut response)
}

fn post_json<TRequest: Serialize, TResponse: DeserializeOwned>(
    url: &str,
    enrollment: &RemoteEnrollmentState,
    session: &RemoteSessionState,
    payload: &TRequest,
) -> Result<TResponse, ProviderJobControlError> {
    let request = ureq::post(url)
        .config()
        .timeout_global(Some(JOB_HTTP_TIMEOUT))
        .http_status_as_error(false)
        .build();
    let mut response = request
        .header(
            "Authorization",
            &format!("Bearer {}", enrollment.credential),
        )
        .header("X-Burd-Session-Token", &session.resume_token)
        .header("X-Burd-Device-Id", &enrollment.device_id)
        .send_json(payload)
        .map_err(|error| ProviderJobControlError::new("transport", error.to_string()))?;
    parse_response(&mut response)
}

fn parse_response<T: DeserializeOwned>(
    response: &mut ureq::http::Response<ureq::Body>,
) -> Result<T, ProviderJobControlError> {
    let status = response.status();
    let value = response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(|error| ProviderJobControlError::new("contract", error.to_string()))?;
    if !status.is_success() {
        let kind = value["error"]["code"].as_str().unwrap_or("rejected");
        return Err(ProviderJobControlError::new(
            match kind {
                "unauthorized" => "unauthorized",
                "revoked" => "revoked",
                "expired" => "expired",
                "conflict" => "conflict",
                "not_found" => "not_found",
                _ if status.as_u16() >= 500 => "server_error",
                _ => "rejected",
            },
            value["error"]["message"]
                .as_str()
                .unwrap_or("Control Plane rejected provider job request"),
        ));
    }
    serde_json::from_value(value)
        .map_err(|error| ProviderJobControlError::new("contract", error.to_string()))
}

#[derive(Clone, Copy)]
pub struct ProviderJobWorkerConfig {
    pub poll_interval: Duration,
    pub shutdown_grace: Duration,
    pub completed_assignment_memory: usize,
}

impl Default for ProviderJobWorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval: JOB_POLL_INTERVAL,
            shutdown_grace: JOB_SHUTDOWN_GRACE,
            completed_assignment_memory: COMPLETED_ASSIGNMENT_MEMORY,
        }
    }
}

pub async fn run_worker(
    control_plane: Arc<dyn ProviderJobControlPlane>,
    executor: Arc<dyn ProviderJobExecutor>,
    shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    run_worker_with_config(
        control_plane,
        executor,
        shutdown,
        ProviderJobWorkerConfig::default(),
    )
    .await
}

pub async fn run_worker_with_config(
    control_plane: Arc<dyn ProviderJobControlPlane>,
    executor: Arc<dyn ProviderJobExecutor>,
    shutdown: watch::Receiver<bool>,
    config: ProviderJobWorkerConfig,
) -> Result<(), String> {
    run_worker_with_data_plane_and_config(
        control_plane,
        executor,
        Arc::new(NoArtifactProviderJobDataPlane),
        shutdown,
        config,
    )
    .await
}

pub async fn run_worker_with_data_plane_and_config(
    control_plane: Arc<dyn ProviderJobControlPlane>,
    executor: Arc<dyn ProviderJobExecutor>,
    data_plane: Arc<dyn ProviderJobDataPlane>,
    mut shutdown: watch::Receiver<bool>,
    config: ProviderJobWorkerConfig,
) -> Result<(), String> {
    let mut completed = CompletedAssignments::new(config.completed_assignment_memory);
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let Some(poll_result) = blocking_control_call(
            {
                let control_plane = Arc::clone(&control_plane);
                move || control_plane.next_job()
            },
            &mut shutdown,
            config.shutdown_grace,
        )
        .await?
        else {
            return Ok(());
        };
        let poll = match poll_result {
            Ok(poll) => poll,
            Err(error) => {
                log_worker_event("provider_job_poll_failed", None, None, error.kind());
                wait_for_poll_or_shutdown(config.poll_interval, &mut shutdown).await;
                continue;
            }
        };
        if poll.response.job.is_none() {
            if validate_next_job_execution_response(&poll.response).is_err() {
                log_worker_event("provider_job_bundle_rejected", None, None, "partial_bundle");
            }
            wait_for_poll_or_shutdown(config.poll_interval, &mut shutdown).await;
            continue;
        }
        let assignment = match validated_assignment(&poll.context, poll.response) {
            Ok(assignment) => assignment,
            Err((job_id, lease_id, kind)) => {
                log_worker_event(
                    "provider_job_bundle_rejected",
                    job_id.as_deref(),
                    lease_id.as_deref(),
                    kind,
                );
                wait_for_poll_or_shutdown(config.poll_interval, &mut shutdown).await;
                continue;
            }
        };
        let assignment_key = format!("{}:{}", assignment.job.job_id, assignment.lease.lease_id);
        if completed.contains(&assignment_key) {
            log_worker_event(
                "provider_job_replay_rejected",
                Some(&assignment.job.job_id),
                Some(&assignment.lease.lease_id),
                "assignment_replay",
            );
            wait_for_poll_or_shutdown(config.poll_interval, &mut shutdown).await;
            continue;
        }
        let disposition = run_assignment(
            Arc::clone(&control_plane),
            Arc::clone(&executor),
            Arc::clone(&data_plane),
            poll.context,
            assignment,
            &mut shutdown,
            config.shutdown_grace,
        )
        .await?;
        completed.insert(assignment_key);
        if disposition == AssignmentDisposition::Shutdown {
            return Ok(());
        }
    }
}

fn validated_assignment(
    context: &ProviderJobWorkerContext,
    response: NextJobResponse,
) -> Result<ProviderJobAssignment, (Option<String>, Option<String>, &'static str)> {
    let job_id = response.job.as_ref().map(|job| job.job_id.clone());
    let lease_id = response.lease.as_ref().map(|lease| lease.lease_id.clone());
    validate_next_job_execution_response(&response)
        .map_err(|_| (job_id.clone(), lease_id.clone(), "bundle_contract"))?;
    let NextJobResponse {
        job: Some(job),
        data_plane: Some(data_plane),
        lease: Some(lease),
        execution: Some(execution),
        ..
    } = response
    else {
        return Err((job_id, lease_id, "partial_bundle"));
    };
    validate_provider_job_execution_bundle(&job, &lease, &data_plane, &execution).map_err(
        |_| {
            (
                Some(job.job_id.clone()),
                Some(lease.lease_id.clone()),
                "bundle_contract",
            )
        },
    )?;
    if job.provider_id != context.provider_id
        || job.device_id != context.device_id
        || job.session_id != context.session_id
    {
        return Err((Some(job.job_id), Some(lease.lease_id), "session_binding"));
    }
    if !context
        .gpu_uuids
        .iter()
        .any(|gpu_uuid| gpu_uuid.eq_ignore_ascii_case(&job.gpu_uuid))
    {
        return Err((Some(job.job_id), Some(lease.lease_id), "gpu_binding"));
    }
    if assignment_deadline(&job, &lease, &data_plane).is_err() {
        return Err((Some(job.job_id), Some(lease.lease_id), "assignment_expired"));
    }
    Ok(ProviderJobAssignment {
        job,
        lease,
        data_plane,
        execution,
        workspace: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssignmentDisposition {
    Continue,
    Shutdown,
}

async fn run_assignment(
    control_plane: Arc<dyn ProviderJobControlPlane>,
    executor: Arc<dyn ProviderJobExecutor>,
    data_plane: Arc<dyn ProviderJobDataPlane>,
    context: ProviderJobWorkerContext,
    mut assignment: ProviderJobAssignment,
    shutdown: &mut watch::Receiver<bool>,
    shutdown_grace: Duration,
) -> Result<AssignmentDisposition, String> {
    let deadline =
        match assignment_deadline(&assignment.job, &assignment.lease, &assignment.data_plane) {
            Ok(deadline) => deadline,
            Err(_) => return Ok(AssignmentDisposition::Continue),
        };
    let job_id = assignment.job.job_id.clone();
    let lease_id = assignment.lease.lease_id.clone();
    let Some(accepted) = blocking_control_call(
        {
            let control_plane = Arc::clone(&control_plane);
            let context = context.clone();
            let job_id = job_id.clone();
            let lease_id = lease_id.clone();
            move || {
                control_plane.accept_job(
                    &context,
                    &job_id,
                    &AcceptJobRequest {
                        lease_id,
                        status_message: Some("provider worker accepted assignment".to_string()),
                    },
                )
            }
        },
        shutdown,
        shutdown_grace,
    )
    .await?
    else {
        return Ok(AssignmentDisposition::Shutdown);
    };
    match accepted {
        Err(error) => {
            log_worker_event(
                "provider_job_accept_failed",
                Some(&job_id),
                Some(&assignment.lease.lease_id),
                error.kind(),
            );
            return Ok(AssignmentDisposition::Continue);
        }
        Ok(response) if !job_response_is_bound(&response, &context, &job_id, "accepted") => {
            log_worker_event(
                "provider_job_accept_failed",
                Some(&job_id),
                Some(&assignment.lease.lease_id),
                "contract",
            );
            return Ok(AssignmentDisposition::Continue);
        }
        Ok(_) => {}
    }

    let Some(provisioning) = report_bound_event(
        Arc::clone(&control_plane),
        &context,
        &job_id,
        JobEventTransition {
            sequence: 1,
            event_type: "provisioning",
            expected_status: "provisioning",
        },
        shutdown,
        shutdown_grace,
    )
    .await?
    else {
        return Ok(AssignmentDisposition::Shutdown);
    };
    if let Err(error) = provisioning {
        return submit_failed_best_effort(
            Arc::clone(&control_plane),
            &context,
            &job_id,
            error,
            shutdown,
            shutdown_grace,
        )
        .await;
    }

    let cancellation = JobCancellation::default();
    let prepared = run_provider_stage(
        {
            let data_plane = Arc::clone(&data_plane);
            let assignment = assignment.clone();
            let cancellation = cancellation.clone();
            move || data_plane.prepare_workspace(&assignment, &cancellation)
        },
        &cancellation,
        deadline,
        shutdown,
        shutdown_grace,
    )
    .await?;
    match prepared {
        ProviderStage::Shutdown => return Ok(AssignmentDisposition::Shutdown),
        ProviderStage::Deadline => {
            return submit_failed_best_effort(
                Arc::clone(&control_plane),
                &context,
                &job_id,
                ProviderJobExecutionError::new(
                    "execution_deadline_exceeded",
                    "provider job data-plane preparation exceeded its authoritative deadline",
                ),
                shutdown,
                shutdown_grace,
            )
            .await;
        }
        ProviderStage::Completed(Err(error)) => {
            return submit_failed_best_effort(
                Arc::clone(&control_plane),
                &context,
                &job_id,
                error,
                shutdown,
                shutdown_grace,
            )
            .await;
        }
        ProviderStage::Completed(Ok(workspace)) => assignment.workspace = workspace,
    }

    let Some(running) = report_bound_event(
        Arc::clone(&control_plane),
        &context,
        &job_id,
        JobEventTransition {
            sequence: 2,
            event_type: "running",
            expected_status: "running",
        },
        shutdown,
        shutdown_grace,
    )
    .await?
    else {
        let _ = cleanup_assignment_workspace(&*data_plane, &assignment);
        return Ok(AssignmentDisposition::Shutdown);
    };
    if let Err(error) = running {
        let error = cleanup_or_replace(&*data_plane, &assignment, error);
        return submit_failed_best_effort(
            Arc::clone(&control_plane),
            &context,
            &job_id,
            error,
            shutdown,
            shutdown_grace,
        )
        .await;
    }

    let execution = run_provider_stage(
        {
            let executor = Arc::clone(&executor);
            let assignment = assignment.clone();
            let cancellation = cancellation.clone();
            move || executor.execute(assignment, cancellation)
        },
        &cancellation,
        deadline,
        shutdown,
        shutdown_grace,
    )
    .await?;

    let mut outcome = match execution {
        ProviderStage::Shutdown => {
            let _ = cleanup_assignment_workspace(&*data_plane, &assignment);
            return Ok(AssignmentDisposition::Shutdown);
        }
        ProviderStage::Deadline => {
            let error = cleanup_or_replace(
                &*data_plane,
                &assignment,
                ProviderJobExecutionError::new(
                    "execution_deadline_exceeded",
                    "provider job execution exceeded its authoritative deadline",
                ),
            );
            return submit_failed_best_effort(
                Arc::clone(&control_plane),
                &context,
                &job_id,
                error,
                shutdown,
                shutdown_grace,
            )
            .await;
        }
        ProviderStage::Completed(Err(error)) => {
            let error = cleanup_or_replace(&*data_plane, &assignment, error);
            return submit_failed_best_effort(
                Arc::clone(&control_plane),
                &context,
                &job_id,
                error,
                shutdown,
                shutdown_grace,
            )
            .await;
        }
        ProviderStage::Completed(Ok(outcome)) => outcome,
    };

    if !outcome.metrics.is_object() || !outcome.result_artifacts.is_empty() {
        let error = cleanup_or_replace(
            &*data_plane,
            &assignment,
            ProviderJobExecutionError::new(
                "executor_contract_invalid",
                "provider job executor returned invalid metrics or artifact metadata",
            ),
        );
        return submit_failed_best_effort(
            Arc::clone(&control_plane),
            &context,
            &job_id,
            error,
            shutdown,
            shutdown_grace,
        )
        .await;
    }

    let Some(uploading) = report_bound_event(
        Arc::clone(&control_plane),
        &context,
        &job_id,
        JobEventTransition {
            sequence: 3,
            event_type: "uploading",
            expected_status: "uploading",
        },
        shutdown,
        shutdown_grace,
    )
    .await?
    else {
        let _ = cleanup_assignment_workspace(&*data_plane, &assignment);
        return Ok(AssignmentDisposition::Shutdown);
    };
    if let Err(error) = uploading {
        let error = cleanup_or_replace(&*data_plane, &assignment, error);
        return submit_failed_best_effort(
            Arc::clone(&control_plane),
            &context,
            &job_id,
            error,
            shutdown,
            shutdown_grace,
        )
        .await;
    }

    let uploaded = run_provider_stage(
        {
            let data_plane = Arc::clone(&data_plane);
            let assignment = assignment.clone();
            let cancellation = cancellation.clone();
            move || data_plane.upload_outputs(&assignment, &cancellation)
        },
        &cancellation,
        deadline,
        shutdown,
        shutdown_grace,
    )
    .await?;
    let artifacts = match uploaded {
        ProviderStage::Shutdown => {
            let _ = cleanup_assignment_workspace(&*data_plane, &assignment);
            return Ok(AssignmentDisposition::Shutdown);
        }
        ProviderStage::Deadline => {
            let error = ProviderJobExecutionError::new(
                "execution_deadline_exceeded",
                "provider job artifact upload exceeded its authoritative deadline",
            );
            let error = cleanup_or_replace(&*data_plane, &assignment, error);
            return submit_failed_best_effort(
                Arc::clone(&control_plane),
                &context,
                &job_id,
                error,
                shutdown,
                shutdown_grace,
            )
            .await;
        }
        ProviderStage::Completed(Err(error)) => {
            let error = cleanup_or_replace(&*data_plane, &assignment, error);
            return submit_failed_best_effort(
                Arc::clone(&control_plane),
                &context,
                &job_id,
                error,
                shutdown,
                shutdown_grace,
            )
            .await;
        }
        ProviderStage::Completed(Ok(artifacts)) => artifacts,
    };
    outcome.result_artifacts = artifacts;
    if let Err(error) = cleanup_assignment_workspace(&*data_plane, &assignment) {
        return submit_failed_best_effort(
            Arc::clone(&control_plane),
            &context,
            &job_id,
            error,
            shutdown,
            shutdown_grace,
        )
        .await;
    }
    submit_outcome(
        control_plane,
        &context,
        &job_id,
        outcome,
        shutdown,
        shutdown_grace,
    )
    .await
}

enum ProviderStage<T> {
    Completed(Result<T, ProviderJobExecutionError>),
    Shutdown,
    Deadline,
}

async fn run_provider_stage<T, F>(
    operation: F,
    cancellation: &JobCancellation,
    deadline: DateTime<Utc>,
    shutdown: &mut watch::Receiver<bool>,
    shutdown_grace: Duration,
) -> Result<ProviderStage<T>, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ProviderJobExecutionError> + Send + 'static,
{
    let remaining = (deadline - Utc::now()).to_std().unwrap_or(Duration::ZERO);
    if remaining.is_zero() {
        return Ok(ProviderStage::Deadline);
    }
    let mut task = tokio::task::spawn_blocking(operation);
    tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => {
            cancellation.cancel();
            finish_cancelled_task(&mut task, shutdown_grace).await;
            Ok(ProviderStage::Shutdown)
        }
        _ = tokio::time::sleep(remaining) => {
            cancellation.cancel();
            finish_cancelled_task(&mut task, shutdown_grace).await;
            Ok(ProviderStage::Deadline)
        }
        result = &mut task => result
            .map(ProviderStage::Completed)
            .map_err(|_| "provider job blocking task failed".to_string()),
    }
}

fn cleanup_assignment_workspace(
    data_plane: &dyn ProviderJobDataPlane,
    assignment: &ProviderJobAssignment,
) -> Result<(), ProviderJobExecutionError> {
    assignment
        .workspace
        .as_ref()
        .map_or(Ok(()), |workspace| data_plane.cleanup_workspace(workspace))
}

fn cleanup_or_replace(
    data_plane: &dyn ProviderJobDataPlane,
    assignment: &ProviderJobAssignment,
    original: ProviderJobExecutionError,
) -> ProviderJobExecutionError {
    cleanup_assignment_workspace(data_plane, assignment)
        .err()
        .unwrap_or(original)
}

fn job_response_is_bound(
    response: &JobResponse,
    context: &ProviderJobWorkerContext,
    job_id: &str,
    status: &str,
) -> bool {
    job_record_is_bound(&response.job, context, job_id, status)
}

fn job_record_is_bound(
    job: &burd_protocol::JobRecord,
    context: &ProviderJobWorkerContext,
    job_id: &str,
    status: &str,
) -> bool {
    job.job_id == job_id
        && job.provider_id == context.provider_id
        && job.device_id == context.device_id
        && job.session_id == context.session_id
        && job.status == status
}

fn event_response_is_bound(
    response: &JobEventResponse,
    context: &ProviderJobWorkerContext,
    job_id: &str,
    sequence: u64,
    event_type: &str,
    status: &str,
) -> bool {
    response.event.job_id == job_id
        && response.event.provider_id == context.provider_id
        && response.event.device_id == context.device_id
        && response.event.session_id == context.session_id
        && response.event.sequence == sequence
        && response.event.event_type == event_type
        && job_record_is_bound(&response.job, context, job_id, status)
}

async fn report_event(
    control_plane: Arc<dyn ProviderJobControlPlane>,
    context: &ProviderJobWorkerContext,
    job_id: &str,
    sequence: u64,
    event_type: &'static str,
    shutdown: &mut watch::Receiver<bool>,
    shutdown_grace: Duration,
) -> Result<Option<Result<JobEventResponse, ProviderJobControlError>>, String> {
    let context = context.clone();
    let job_id = job_id.to_string();
    blocking_control_call(
        move || {
            control_plane.record_event(
                &context,
                &job_id,
                &JobEventRequest {
                    sequence,
                    event_type: event_type.to_string(),
                    progress_percent: None,
                    message: Some(format!("provider worker entered {event_type}")),
                    metadata: json!({}),
                    occurred_at: Some(Utc::now().to_rfc3339()),
                },
            )
        },
        shutdown,
        shutdown_grace,
    )
    .await
}

#[derive(Clone, Copy)]
struct JobEventTransition {
    sequence: u64,
    event_type: &'static str,
    expected_status: &'static str,
}

async fn report_bound_event(
    control_plane: Arc<dyn ProviderJobControlPlane>,
    context: &ProviderJobWorkerContext,
    job_id: &str,
    transition: JobEventTransition,
    shutdown: &mut watch::Receiver<bool>,
    shutdown_grace: Duration,
) -> Result<Option<Result<(), ProviderJobExecutionError>>, String> {
    let Some(result) = report_event(
        control_plane,
        context,
        job_id,
        transition.sequence,
        transition.event_type,
        shutdown,
        shutdown_grace,
    )
    .await?
    else {
        return Ok(None);
    };
    let response = match result {
        Ok(response) => response,
        Err(error) => {
            log_worker_event(
                "provider_job_event_failed",
                Some(job_id),
                None,
                error.kind(),
            );
            return Ok(Some(Err(ProviderJobExecutionError::new(
                "state_reporting_failed",
                "provider job state reporting failed",
            ))));
        }
    };
    if event_response_is_bound(
        &response,
        context,
        job_id,
        transition.sequence,
        transition.event_type,
        transition.expected_status,
    ) {
        Ok(Some(Ok(())))
    } else {
        Ok(Some(Err(ProviderJobExecutionError::new(
            "state_reporting_failed",
            "provider job state response violated its contract",
        ))))
    }
}

async fn submit_outcome(
    control_plane: Arc<dyn ProviderJobControlPlane>,
    context: &ProviderJobWorkerContext,
    job_id: &str,
    outcome: ProviderJobExecutionOutcome,
    shutdown: &mut watch::Receiver<bool>,
    shutdown_grace: Duration,
) -> Result<AssignmentDisposition, String> {
    let request = SubmitJobResultRequest {
        status: "succeeded".to_string(),
        result_artifacts: outcome.result_artifacts,
        metrics: outcome.metrics,
        error_code: None,
        error_message: None,
        completed_at: Some(Utc::now().to_rfc3339()),
    };
    submit_result_best_effort(
        control_plane,
        context,
        job_id,
        request,
        shutdown,
        shutdown_grace,
    )
    .await
}

async fn submit_failed_best_effort(
    control_plane: Arc<dyn ProviderJobControlPlane>,
    context: &ProviderJobWorkerContext,
    job_id: &str,
    error: ProviderJobExecutionError,
    shutdown: &mut watch::Receiver<bool>,
    shutdown_grace: Duration,
) -> Result<AssignmentDisposition, String> {
    let request = SubmitJobResultRequest {
        status: "failed".to_string(),
        result_artifacts: Vec::new(),
        metrics: json!({}),
        error_code: Some(error.code().to_string()),
        error_message: Some(error.message().to_string()),
        completed_at: Some(Utc::now().to_rfc3339()),
    };
    submit_result_best_effort(
        control_plane,
        context,
        job_id,
        request,
        shutdown,
        shutdown_grace,
    )
    .await
}

async fn submit_result_best_effort(
    control_plane: Arc<dyn ProviderJobControlPlane>,
    context: &ProviderJobWorkerContext,
    job_id: &str,
    request: SubmitJobResultRequest,
    shutdown: &mut watch::Receiver<bool>,
    shutdown_grace: Duration,
) -> Result<AssignmentDisposition, String> {
    let context = context.clone();
    let expected_context = context.clone();
    let expected_status = request.status.clone();
    let job_id = job_id.to_string();
    let task_job_id = job_id.clone();
    let Some(result) = blocking_control_call(
        move || control_plane.submit_result(&context, &task_job_id, &request),
        shutdown,
        shutdown_grace,
    )
    .await?
    else {
        return Ok(AssignmentDisposition::Shutdown);
    };
    match result {
        Ok(response)
            if job_record_is_bound(&response.job, &expected_context, &job_id, &expected_status) => {
        }
        Ok(_) => log_worker_event(
            "provider_job_result_failed",
            Some(&job_id),
            None,
            "contract",
        ),
        Err(error) => log_worker_event(
            "provider_job_result_failed",
            Some(&job_id),
            None,
            error.kind(),
        ),
    }
    Ok(AssignmentDisposition::Continue)
}

fn assignment_deadline(
    job: &burd_protocol::JobRecord,
    lease: &burd_protocol::JobLeaseRecord,
    data_plane: &burd_protocol::JobDataPlaneGrant,
) -> Result<DateTime<Utc>, String> {
    let now = Utc::now();
    let lease_expiry = parse_utc(&lease.expires_at)?;
    let credential_expiry = parse_utc(&data_plane.credential_expires_at)?;
    if lease_expiry <= now || credential_expiry <= now {
        return Err("provider job assignment is expired".to_string());
    }
    let timeout_deadline = now + chrono::Duration::seconds(i64::from(job.timeout_seconds));
    Ok(lease_expiry.min(credential_expiry).min(timeout_deadline))
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| "provider job assignment contains an invalid timestamp".to_string())
}

async fn finish_cancelled_task<T>(
    task: &mut tokio::task::JoinHandle<Result<T, ProviderJobExecutionError>>,
    grace: Duration,
) where
    T: Send + 'static,
{
    if tokio::time::timeout(grace, &mut *task).await.is_err() {
        task.abort();
        log_worker_event(
            "provider_job_shutdown_grace_exceeded",
            None,
            None,
            "executor_shutdown_timeout",
        );
    }
}

async fn blocking_control_call<T, F>(
    operation: F,
    shutdown: &mut watch::Receiver<bool>,
    shutdown_grace: Duration,
) -> Result<Option<Result<T, ProviderJobControlError>>, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ProviderJobControlError> + Send + 'static,
{
    let mut task = tokio::task::spawn_blocking(operation);
    tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => {
            if tokio::time::timeout(shutdown_grace, &mut task).await.is_err() {
                task.abort();
            }
            Ok(None)
        }
        result = &mut task => {
            result
                .map(Some)
                .map_err(|_| "provider job control-plane task failed".to_string())
        }
    }
}

async fn wait_for_poll_or_shutdown(interval: Duration, shutdown: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::time::sleep(interval) => {}
        _ = wait_for_shutdown(shutdown) => {}
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

fn log_worker_event(
    event: &'static str,
    job_id: Option<&str>,
    lease_id: Option<&str>,
    failure_kind: &'static str,
) {
    eprintln!(
        "{}",
        worker_event_value(event, job_id, lease_id, failure_kind)
    );
}

fn worker_event_value(
    event: &'static str,
    job_id: Option<&str>,
    lease_id: Option<&str>,
    failure_kind: &'static str,
) -> serde_json::Value {
    json!({
        "event": event,
        "job_id": job_id,
        "lease_id": lease_id,
        "failure_kind": failure_kind,
    })
}

struct CompletedAssignments {
    capacity: usize,
    order: VecDeque<String>,
    entries: HashSet<String>,
}

impl CompletedAssignments {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            entries: HashSet::new(),
        }
    }

    fn contains(&self, key: &str) -> bool {
        self.entries.contains(key)
    }

    fn insert(&mut self, key: String) {
        if !self.entries.insert(key.clone()) {
            return;
        }
        self.order.push_back(key);
        while self.order.len() > self.capacity {
            if let Some(expired) = self.order.pop_front() {
                self.entries.remove(&expired);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::{
        JOB_DATA_PLANE_GRANT_VERSION, JOB_LEASE_SCHEMA_VERSION, JOB_SCHEMA_VERSION,
        JobDataPlaneGrant, JobLeaseRecord, JobRecord, PROVIDER_JOB_EXECUTION_POLICY_VERSION,
        PROVIDER_JOB_EXECUTION_SCHEMA_VERSION, ProviderJobCancellationPolicy,
        ProviderJobCleanupPolicy, ProviderJobExecutionSpec, ProviderJobExecutionState,
        ProviderJobRuntimePolicy,
    };
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FakeControlPlane {
        context: ProviderJobWorkerContext,
        state: Mutex<FakeControlState>,
    }

    #[derive(Default)]
    struct FakeControlState {
        polls: VecDeque<NextJobResponse>,
        poll_count: usize,
        accepts: usize,
        accepted_lease_ids: Vec<String>,
        events: Vec<String>,
        results: Vec<SubmitJobResultRequest>,
    }

    impl FakeControlPlane {
        fn new(polls: Vec<NextJobResponse>) -> Self {
            Self {
                context: worker_context(),
                state: Mutex::new(FakeControlState {
                    polls: polls.into(),
                    ..FakeControlState::default()
                }),
            }
        }

        fn snapshot(&self) -> (usize, usize, Vec<String>, Vec<SubmitJobResultRequest>) {
            let state = self.state.lock().unwrap();
            (
                state.poll_count,
                state.accepts,
                state.events.clone(),
                state.results.clone(),
            )
        }

        fn accepted_lease_ids(&self) -> Vec<String> {
            self.state.lock().unwrap().accepted_lease_ids.clone()
        }
    }

    impl ProviderJobControlPlane for FakeControlPlane {
        fn next_job(&self) -> Result<ProviderJobPoll, ProviderJobControlError> {
            let mut state = self.state.lock().unwrap();
            state.poll_count += 1;
            let response = state.polls.pop_front().unwrap_or(NextJobResponse {
                request_id: "req_empty".to_string(),
                job: None,
                data_plane: None,
                lease: None,
                execution: None,
            });
            Ok(ProviderJobPoll {
                context: self.context.clone(),
                response,
            })
        }

        fn accept_job(
            &self,
            _context: &ProviderJobWorkerContext,
            job_id: &str,
            request: &AcceptJobRequest,
        ) -> Result<JobResponse, ProviderJobControlError> {
            let mut state = self.state.lock().unwrap();
            state.accepts += 1;
            state.accepted_lease_ids.push(request.lease_id.clone());
            drop(state);
            Ok(JobResponse {
                request_id: "req_accept".to_string(),
                job: job_with_status(job_id, "accepted"),
            })
        }

        fn record_event(
            &self,
            context: &ProviderJobWorkerContext,
            job_id: &str,
            request: &JobEventRequest,
        ) -> Result<JobEventResponse, ProviderJobControlError> {
            self.state
                .lock()
                .unwrap()
                .events
                .push(request.event_type.clone());
            let status = match request.event_type.as_str() {
                "provisioning" => "provisioning",
                "started" | "running" => "running",
                "uploading" => "uploading",
                _ => "accepted",
            };
            Ok(JobEventResponse {
                request_id: "req_event".to_string(),
                event: burd_protocol::JobEventRecord {
                    event_id: format!("event_{}", request.sequence),
                    job_id: job_id.to_string(),
                    provider_id: context.provider_id.clone(),
                    device_id: context.device_id.clone(),
                    session_id: context.session_id.clone(),
                    sequence: request.sequence,
                    schema_version: burd_protocol::JOB_EVENT_SCHEMA_VERSION.to_string(),
                    event_type: request.event_type.clone(),
                    progress_percent: request.progress_percent,
                    message: request.message.clone(),
                    metadata: request.metadata.clone(),
                    occurred_at: request
                        .occurred_at
                        .clone()
                        .unwrap_or_else(|| Utc::now().to_rfc3339()),
                    server_received_at: Utc::now().to_rfc3339(),
                },
                job: job_with_status(job_id, status),
            })
        }

        fn submit_result(
            &self,
            _context: &ProviderJobWorkerContext,
            job_id: &str,
            request: &SubmitJobResultRequest,
        ) -> Result<SubmitJobResultResponse, ProviderJobControlError> {
            self.state.lock().unwrap().results.push(request.clone());
            Ok(SubmitJobResultResponse {
                request_id: "req_result".to_string(),
                job: job_with_status(job_id, &request.status),
            })
        }
    }

    #[derive(Clone, Copy)]
    enum ExecutorBehavior {
        Succeed,
        Fail,
        WaitForCancellation,
    }

    struct FakeProviderJobExecutor {
        behavior: ExecutorBehavior,
        calls: AtomicUsize,
        observed_cancellation: AtomicBool,
    }

    impl FakeProviderJobExecutor {
        fn new(behavior: ExecutorBehavior) -> Self {
            Self {
                behavior,
                calls: AtomicUsize::new(0),
                observed_cancellation: AtomicBool::new(false),
            }
        }
    }

    impl ProviderJobExecutor for FakeProviderJobExecutor {
        fn execute(
            &self,
            _assignment: ProviderJobAssignment,
            cancellation: JobCancellation,
        ) -> Result<ProviderJobExecutionOutcome, ProviderJobExecutionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.behavior {
                ExecutorBehavior::Succeed => Ok(ProviderJobExecutionOutcome::new(
                    Vec::new(),
                    json!({"contract_only": true, "executor": "fake"}),
                )),
                ExecutorBehavior::Fail => Err(ProviderJobExecutionError::new(
                    "fake_executor_failed",
                    "deterministic fake executor failure",
                )),
                ExecutorBehavior::WaitForCancellation => {
                    for _ in 0..5_000 {
                        if cancellation.requested() {
                            self.observed_cancellation.store(true, Ordering::SeqCst);
                            return Err(ProviderJobExecutionError::new(
                                "execution_cancelled",
                                "fake executor observed cancellation",
                            ));
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(ProviderJobExecutionError::new(
                        "fake_executor_timeout",
                        "fake executor did not receive cancellation",
                    ))
                }
            }
        }
    }

    fn worker_context() -> ProviderJobWorkerContext {
        ProviderJobWorkerContext {
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            gpu_uuids: vec!["GPU-test".to_string()],
        }
    }

    fn assignment_response() -> NextJobResponse {
        let now = Utc::now();
        let lease_expires_at = (now + chrono::Duration::minutes(5)).to_rfc3339();
        let credential_expires_at = (now + chrono::Duration::minutes(20)).to_rfc3339();
        let job = JobRecord {
            job_id: "job_1".to_string(),
            client_job_id: None,
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            schema_version: JOB_SCHEMA_VERSION.to_string(),
            workload_type: "llm_realtime_api".to_string(),
            template_id: "llm_inference".to_string(),
            image_ref: format!("ghcr.io/burd/llm@sha256:{}", "a".repeat(64)),
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
            timeout_seconds: 900,
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
            credential: "jobcred_never_log_this".to_string(),
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
            cancellation: ProviderJobCancellationPolicy::v1(),
            cleanup: ProviderJobCleanupPolicy::v1(),
        };
        NextJobResponse {
            request_id: "req_next".to_string(),
            job: Some(job),
            data_plane: Some(data_plane),
            lease: Some(lease),
            execution: Some(execution),
        }
    }

    fn job_with_status(job_id: &str, status: &str) -> JobRecord {
        let mut job = assignment_response().job.unwrap();
        job.job_id = job_id.to_string();
        job.status = status.to_string();
        job
    }

    fn test_config() -> ProviderJobWorkerConfig {
        ProviderJobWorkerConfig {
            poll_interval: Duration::from_millis(2),
            shutdown_grace: Duration::from_millis(250),
            completed_assignment_memory: 8,
        }
    }

    async fn wait_until(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !predicate() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("worker condition timed out");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn success_uses_the_authoritative_state_flow() {
        let control = Arc::new(FakeControlPlane::new(vec![assignment_response()]));
        let executor = Arc::new(FakeProviderJobExecutor::new(ExecutorBehavior::Succeed));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker = tokio::spawn(run_worker_with_config(
            control.clone(),
            executor.clone(),
            shutdown_rx,
            test_config(),
        ));
        wait_until(|| !control.snapshot().3.is_empty()).await;
        shutdown_tx.send(true).unwrap();
        worker.await.unwrap().unwrap();

        let (_, accepts, events, results) = control.snapshot();
        assert_eq!(accepts, 1);
        assert_eq!(control.accepted_lease_ids(), ["lease_1"]);
        assert_eq!(events, ["provisioning", "running", "uploading"]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "succeeded");
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn executor_failure_is_submitted_as_a_failed_result() {
        let control = Arc::new(FakeControlPlane::new(vec![assignment_response()]));
        let executor = Arc::new(FakeProviderJobExecutor::new(ExecutorBehavior::Fail));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker = tokio::spawn(run_worker_with_config(
            control.clone(),
            executor,
            shutdown_rx,
            test_config(),
        ));
        wait_until(|| !control.snapshot().3.is_empty()).await;
        shutdown_tx.send(true).unwrap();
        worker.await.unwrap().unwrap();

        let (_, accepts, events, results) = control.snapshot();
        assert_eq!(accepts, 1);
        assert_eq!(events, ["provisioning", "running"]);
        assert_eq!(results[0].status, "failed");
        assert_eq!(
            results[0].error_code.as_deref(),
            Some("fake_executor_failed")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalid_bundle_fails_closed_before_acceptance() {
        let mut response = assignment_response();
        response.execution.as_mut().unwrap().session_id = "session_other".to_string();
        let control = Arc::new(FakeControlPlane::new(vec![response]));
        let executor = Arc::new(FakeProviderJobExecutor::new(ExecutorBehavior::Succeed));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker = tokio::spawn(run_worker_with_config(
            control.clone(),
            executor.clone(),
            shutdown_rx,
            test_config(),
        ));
        wait_until(|| control.snapshot().0 >= 1).await;
        shutdown_tx.send(true).unwrap();
        worker.await.unwrap().unwrap();
        assert_eq!(control.snapshot().1, 0);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn expired_lease_fails_closed_before_acceptance() {
        let mut response = assignment_response();
        let expired = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        response.lease.as_mut().unwrap().expires_at = expired.clone();
        response.execution.as_mut().unwrap().lease_expires_at = expired;
        let control = Arc::new(FakeControlPlane::new(vec![response]));
        let executor = Arc::new(FakeProviderJobExecutor::new(ExecutorBehavior::Succeed));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker = tokio::spawn(run_worker_with_config(
            control.clone(),
            executor.clone(),
            shutdown_rx,
            test_config(),
        ));
        wait_until(|| control.snapshot().0 >= 1).await;
        shutdown_tx.send(true).unwrap();
        worker.await.unwrap().unwrap();
        assert_eq!(control.snapshot().1, 0);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_gpu_mismatch_fails_closed_before_acceptance() {
        let mut fake_control = FakeControlPlane::new(vec![assignment_response()]);
        fake_control.context.gpu_uuids = vec!["GPU-other".to_string()];
        let control = Arc::new(fake_control);
        let executor = Arc::new(FakeProviderJobExecutor::new(ExecutorBehavior::Succeed));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker = tokio::spawn(run_worker_with_config(
            control.clone(),
            executor.clone(),
            shutdown_rx,
            test_config(),
        ));
        wait_until(|| control.snapshot().0 >= 1).await;
        shutdown_tx.send(true).unwrap();
        worker.await.unwrap().unwrap();
        assert_eq!(control.snapshot().1, 0);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_cancels_the_active_executor_cooperatively() {
        let control = Arc::new(FakeControlPlane::new(vec![assignment_response()]));
        let executor = Arc::new(FakeProviderJobExecutor::new(
            ExecutorBehavior::WaitForCancellation,
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker = tokio::spawn(run_worker_with_config(
            control,
            executor.clone(),
            shutdown_rx,
            test_config(),
        ));
        wait_until(|| executor.calls.load(Ordering::SeqCst) == 1).await;
        shutdown_tx.send(true).unwrap();
        worker.await.unwrap().unwrap();
        assert!(executor.observed_cancellation.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn active_execution_is_cancelled_at_the_assignment_deadline() {
        let mut response = assignment_response();
        let deadline = (Utc::now() + chrono::Duration::milliseconds(500)).to_rfc3339();
        response.lease.as_mut().unwrap().expires_at = deadline.clone();
        response.execution.as_mut().unwrap().lease_expires_at = deadline;
        let control = Arc::new(FakeControlPlane::new(vec![response]));
        let executor = Arc::new(FakeProviderJobExecutor::new(
            ExecutorBehavior::WaitForCancellation,
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker = tokio::spawn(run_worker_with_config(
            control.clone(),
            executor.clone(),
            shutdown_rx,
            test_config(),
        ));
        wait_until(|| !control.snapshot().3.is_empty()).await;
        shutdown_tx.send(true).unwrap();
        worker.await.unwrap().unwrap();
        let results = control.snapshot().3;
        assert!(executor.observed_cancellation.load(Ordering::SeqCst));
        assert_eq!(results[0].status, "failed");
        assert_eq!(
            results[0].error_code.as_deref(),
            Some("execution_deadline_exceeded")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn replayed_assignment_is_not_executed_twice() {
        let response = assignment_response();
        let control = Arc::new(FakeControlPlane::new(vec![response.clone(), response]));
        let executor = Arc::new(FakeProviderJobExecutor::new(ExecutorBehavior::Succeed));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker = tokio::spawn(run_worker_with_config(
            control.clone(),
            executor.clone(),
            shutdown_rx,
            test_config(),
        ));
        wait_until(|| control.snapshot().0 >= 2).await;
        shutdown_tx.send(true).unwrap();
        worker.await.unwrap().unwrap();
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        assert_eq!(control.snapshot().1, 1);
    }

    #[test]
    fn diagnostics_and_worker_events_do_not_serialize_secret_details() {
        let error = ProviderJobControlError::new(
            "transport",
            "Authorization: Bearer device-secret; jobcred_never_log_this; resume-secret",
        );
        let debug = format!("{error:?}");
        let display = error.to_string();
        let event = worker_event_value(
            "provider_job_poll_failed",
            Some("job_1"),
            Some("lease_1"),
            error.kind(),
        )
        .to_string();
        for output in [debug, display, event] {
            assert!(!output.contains("device-secret"));
            assert!(!output.contains("jobcred_never_log_this"));
            assert!(!output.contains("resume-secret"));
        }
        let unsafe_kind = ProviderJobControlError::new("device-secret", "private detail");
        assert_eq!(unsafe_kind.kind(), "control_plane_failure");
        assert!(!format!("{unsafe_kind:?}").contains("device-secret"));
    }
}
