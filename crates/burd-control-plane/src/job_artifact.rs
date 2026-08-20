use crate::db::Database;
use crate::protocol_negotiation::assert_current_compute_protocol_negotiation;
use crate::remote_session::SessionError;
use burd_protocol::{JobArtifact, sha256_hex};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use tokio_postgres::Transaction;

const MAX_ARTIFACT_BYTES: u64 = 10 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobArtifactDirection {
    Download,
    Upload,
}

#[derive(Clone, Debug)]
pub struct AuthorizedJobArtifact {
    pub artifact: JobArtifact,
}

impl Database {
    pub async fn authorize_job_artifact(
        &self,
        job_id: &str,
        artifact_id: &str,
        credential: &str,
        direction: JobArtifactDirection,
    ) -> Result<AuthorizedJobArtifact, SessionError> {
        validate_id(job_id)?;
        validate_id(artifact_id)?;
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_opt(
                "SELECT session_id, status, input_artifacts_json, expected_outputs_json, job_credential_hash, job_credential_expires_at FROM compute_jobs WHERE job_id = $1",
                &[&job_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("job not found".to_string()))?;
        assert_current_compute_protocol_negotiation(
            &transaction,
            &row.get::<_, String>("session_id"),
        )
        .await?;
        let artifact = authorized_artifact_from_row(&row, artifact_id, credential, direction)?;
        transaction.commit().await?;
        Ok(AuthorizedJobArtifact { artifact })
    }

    pub async fn record_job_artifact_upload(
        &self,
        job_id: &str,
        artifact_id: &str,
        credential: &str,
        sha256: &str,
        size_bytes: u64,
        content_type: Option<&str>,
    ) -> Result<JobArtifact, SessionError> {
        validate_digest(sha256)?;
        if size_bytes > MAX_ARTIFACT_BYTES {
            return Err(SessionError::Invalid(
                "uploaded artifact exceeds the maximum size".to_string(),
            ));
        }
        let mut client = self.connect().await?;
        let transaction = client.transaction().await?;
        let row = transaction
            .query_opt(
                "SELECT session_id, status, input_artifacts_json, expected_outputs_json, job_credential_hash, job_credential_expires_at FROM compute_jobs WHERE job_id = $1 FOR UPDATE",
                &[&job_id],
            )
            .await?
            .ok_or_else(|| SessionError::NotFound("job not found".to_string()))?;
        assert_current_compute_protocol_negotiation(
            &transaction,
            &row.get::<_, String>("session_id"),
        )
        .await?;
        let expected = authorized_artifact_from_row(
            &row,
            artifact_id,
            credential,
            JobArtifactDirection::Upload,
        )?;
        let maximum_size = expected.size_bytes.ok_or_else(|| {
            SessionError::Invalid("expected output size_bytes is required".to_string())
        })?;
        if size_bytes > maximum_size {
            return Err(SessionError::Invalid(
                "uploaded artifact exceeds its declared size limit".to_string(),
            ));
        }
        if let Some(expected_content_type) = expected.content_type.as_deref()
            && content_type != Some(expected_content_type)
        {
            return Err(SessionError::Invalid(
                "uploaded artifact content type does not match its manifest".to_string(),
            ));
        }
        let uploaded_at = Utc::now().to_rfc3339();
        let size_i64 = i64::try_from(size_bytes)
            .map_err(|_| SessionError::Invalid("uploaded artifact size is invalid".to_string()))?;
        transaction
            .execute(
                "INSERT INTO job_artifact_uploads (job_id, artifact_id, object_key, sha256, size_bytes, content_type, uploaded_at) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (job_id, artifact_id) DO UPDATE SET object_key = EXCLUDED.object_key, sha256 = EXCLUDED.sha256, size_bytes = EXCLUDED.size_bytes, content_type = EXCLUDED.content_type, uploaded_at = EXCLUDED.uploaded_at",
                &[
                    &job_id,
                    &artifact_id,
                    &expected.object_key,
                    &sha256,
                    &size_i64,
                    &content_type,
                    &uploaded_at,
                ],
            )
            .await?;
        transaction.commit().await?;
        Ok(JobArtifact {
            artifact_id: expected.artifact_id,
            role: expected.role,
            object_key: expected.object_key,
            sha256: Some(sha256.to_string()),
            size_bytes: Some(size_bytes),
            content_type: expected.content_type,
        })
    }
}

pub(crate) async fn validate_uploaded_job_results(
    transaction: &Transaction<'_>,
    job_id: &str,
    expected_outputs: &[JobArtifact],
    status: &str,
    result_artifacts: &[JobArtifact],
) -> Result<(), SessionError> {
    if status == "failed" {
        return if result_artifacts.is_empty() {
            Ok(())
        } else {
            Err(SessionError::Invalid(
                "failed jobs cannot claim uploaded result artifacts".to_string(),
            ))
        };
    }
    if result_artifacts.len() != expected_outputs.len() {
        return Err(SessionError::Invalid(
            "successful job result does not contain every expected output".to_string(),
        ));
    }
    let expected = expected_outputs
        .iter()
        .map(|artifact| (artifact.artifact_id.as_str(), artifact))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let rows = transaction
        .query(
            "SELECT artifact_id, object_key, sha256, size_bytes, content_type FROM job_artifact_uploads WHERE job_id = $1",
            &[&job_id],
        )
        .await?;
    let uploaded = rows
        .into_iter()
        .map(|row| {
            let artifact_id: String = row.get("artifact_id");
            let size: i64 = row.get("size_bytes");
            (
                artifact_id,
                JobArtifact {
                    artifact_id: row.get("artifact_id"),
                    role: "output".to_string(),
                    object_key: row.get("object_key"),
                    sha256: Some(row.get("sha256")),
                    size_bytes: u64::try_from(size).ok(),
                    content_type: row.get("content_type"),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    for result in result_artifacts {
        let Some(manifest) = expected.get(result.artifact_id.as_str()) else {
            return Err(SessionError::Invalid(
                "job result contains an undeclared output".to_string(),
            ));
        };
        let Some(stored) = uploaded.get(&result.artifact_id) else {
            return Err(SessionError::Invalid(
                "job result references an output that was not uploaded".to_string(),
            ));
        };
        if !seen.insert(result.artifact_id.as_str())
            || result.role != manifest.role
            || result.object_key != manifest.object_key
            || result.sha256 != stored.sha256
            || result.size_bytes != stored.size_bytes
            || result.content_type != manifest.content_type
        {
            return Err(SessionError::Invalid(
                "job result artifact does not match its verified upload".to_string(),
            ));
        }
    }
    Ok(())
}

fn authorized_artifact_from_row(
    row: &tokio_postgres::Row,
    artifact_id: &str,
    credential: &str,
    direction: JobArtifactDirection,
) -> Result<JobArtifact, SessionError> {
    let stored_hash = row
        .get::<_, Option<String>>("job_credential_hash")
        .ok_or(SessionError::Unauthorized)?;
    let expires_at = row
        .get::<_, Option<String>>("job_credential_expires_at")
        .ok_or(SessionError::Unauthorized)?;
    if sha256_hex(credential.as_bytes()) != stored_hash
        || DateTime::parse_from_rfc3339(&expires_at)
            .map_err(|_| SessionError::Invalid("job credential expiry is invalid".to_string()))?
            .with_timezone(&Utc)
            <= Utc::now()
    {
        return Err(SessionError::Unauthorized);
    }
    let status: String = row.get("status");
    let status_allowed = match direction {
        JobArtifactDirection::Download => {
            matches!(status.as_str(), "assigned" | "accepted" | "provisioning")
        }
        JobArtifactDirection::Upload => status == "uploading",
    };
    if !status_allowed {
        return Err(SessionError::Conflict(
            "job state does not allow this artifact transfer".to_string(),
        ));
    }
    let json: String = match direction {
        JobArtifactDirection::Download => row.get("input_artifacts_json"),
        JobArtifactDirection::Upload => row.get("expected_outputs_json"),
    };
    let artifacts = serde_json::from_str::<Vec<JobArtifact>>(&json)
        .map_err(|error| SessionError::Invalid(error.to_string()))?;
    let artifact = artifacts
        .into_iter()
        .find(|artifact| artifact.artifact_id == artifact_id)
        .ok_or_else(|| SessionError::NotFound("job artifact not found".to_string()))?;
    match direction {
        JobArtifactDirection::Download => {
            if artifact.size_bytes.is_none() || artifact.sha256.is_none() {
                return Err(SessionError::Invalid(
                    "input artifact requires size_bytes and sha256".to_string(),
                ));
            }
        }
        JobArtifactDirection::Upload => {
            if artifact.size_bytes.is_none() {
                return Err(SessionError::Invalid(
                    "expected output requires a size_bytes limit".to_string(),
                ));
            }
        }
    }
    Ok(artifact)
}

fn validate_id(value: &str) -> Result<(), SessionError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        Err(SessionError::Invalid(
            "job artifact identifier is invalid".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn validate_digest(value: &str) -> Result<(), SessionError> {
    let valid = value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if valid {
        Ok(())
    } else {
        Err(SessionError::Invalid(
            "uploaded artifact digest is invalid".to_string(),
        ))
    }
}
