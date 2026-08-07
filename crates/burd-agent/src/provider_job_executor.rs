use burd_protocol::{
    JobArtifact, JobDataPlaneGrant, JobLeaseRecord, JobRecord, ProviderJobExecutionSpec,
};
use std::fmt::{self, Display, Formatter};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Complete, locally validated assignment passed to a provider runtime executor.
///
/// This type intentionally does not implement `Debug`: the data-plane grant contains a
/// short-lived credential that must never be written to logs.
#[derive(Clone)]
pub struct ProviderJobAssignment {
    pub job: JobRecord,
    pub lease: JobLeaseRecord,
    pub data_plane: JobDataPlaneGrant,
    pub execution: ProviderJobExecutionSpec,
}

#[derive(Clone, Default)]
pub struct JobCancellation {
    requested: Arc<AtomicBool>,
}

impl JobCancellation {
    pub fn cancel(&self) {
        self.requested.store(true, Ordering::Release);
    }

    pub fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    pub fn ensure_not_cancelled(&self) -> Result<(), ProviderJobExecutionError> {
        if self.requested() {
            Err(ProviderJobExecutionError::new(
                "execution_cancelled",
                "provider job execution was cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct ProviderJobExecutionOutcome {
    pub result_artifacts: Vec<JobArtifact>,
    pub metrics: serde_json::Value,
}

impl ProviderJobExecutionOutcome {
    pub fn new(result_artifacts: Vec<JobArtifact>, metrics: serde_json::Value) -> Self {
        Self {
            result_artifacts,
            metrics,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProviderJobExecutionError {
    code: String,
    message: String,
}

impl fmt::Debug for ProviderJobExecutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderJobExecutionError")
            .field("code", &self.code)
            .finish_non_exhaustive()
    }
}

impl Display for ProviderJobExecutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "provider job execution failed ({})", self.code)
    }
}

impl std::error::Error for ProviderJobExecutionError {}

impl ProviderJobExecutionError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: sanitize_error_code(&code.into()),
            message: sanitize_error_message(&message.into()),
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub trait ProviderJobExecutor: Send + Sync + 'static {
    fn execute(
        &self,
        assignment: ProviderJobAssignment,
        cancellation: JobCancellation,
    ) -> Result<ProviderJobExecutionOutcome, ProviderJobExecutionError>;
}

fn sanitize_error_code(value: &str) -> String {
    let sanitized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        .take(64)
        .collect::<String>();
    let lowercase = sanitized.to_ascii_lowercase();
    if sanitized.is_empty()
        || ["secret", "token", "key", "credential", "bearer", "jobcred"]
            .iter()
            .any(|marker| lowercase.contains(marker))
    {
        "executor_failed".to_string()
    } else {
        sanitized
    }
}

fn sanitize_error_message(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect::<String>();
    for marker in [
        "authorization",
        "bearer",
        "jobcred",
        "credential",
        "secret",
        "token",
        "key",
    ] {
        if sanitized
            .to_ascii_lowercase()
            .contains(&marker.to_ascii_lowercase())
        {
            return "provider job executor returned a redacted error".to_string();
        }
    }
    if sanitized.is_empty() {
        sanitized = "provider job executor failed".to_string();
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_errors_are_bounded_and_redacted() {
        let error = ProviderJobExecutionError::new(
            "bad code!",
            "Authorization: Bearer device-secret and credential=jobcred_secret",
        );
        assert_eq!(error.code(), "badcode");
        assert_eq!(
            error.message(),
            "provider job executor returned a redacted error"
        );
        assert!(!error.message().contains("device-secret"));
        assert!(!error.message().contains("jobcred_secret"));
        assert_eq!(
            ProviderJobExecutionError::new("token_secret", "failed").code(),
            "executor_failed"
        );
    }

    #[test]
    fn cancellation_is_shared_with_the_executor() {
        let cancellation = JobCancellation::default();
        let worker = cancellation.clone();
        assert!(!worker.requested());
        cancellation.cancel();
        assert!(worker.requested());
        assert_eq!(
            worker.ensure_not_cancelled().unwrap_err().code(),
            "execution_cancelled"
        );
    }
}
