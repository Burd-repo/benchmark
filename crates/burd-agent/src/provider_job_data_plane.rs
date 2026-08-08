use crate::provider_job_executor::{
    JobCancellation, ProviderJobAssignment, ProviderJobExecutionError,
    ProviderJobExecutionWorkspace,
};
use crate::remote_enrollment::join_url;
use burd_protocol::{
    JobArtifact, JobArtifactUploadResponse, JobDataPlaneUrl, Sha256Accumulator,
    create_private_directory_all, create_private_file_new, default_state_dir, random_token,
    restrict_private_file,
};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DATA_PLANE_TIMEOUT: Duration = Duration::from_secs(120);
const TRANSFER_BUFFER_BYTES: usize = 64 * 1024;
const MAX_ARTIFACTS: usize = 32;
const MAX_ARTIFACT_BYTES: u64 = 10 * 1024 * 1024 * 1024;

pub trait ProviderJobDataPlane: Send + Sync + 'static {
    fn prepare_workspace(
        &self,
        assignment: &ProviderJobAssignment,
        cancellation: &JobCancellation,
    ) -> Result<Option<ProviderJobExecutionWorkspace>, ProviderJobExecutionError>;

    fn upload_outputs(
        &self,
        assignment: &ProviderJobAssignment,
        cancellation: &JobCancellation,
    ) -> Result<Vec<JobArtifact>, ProviderJobExecutionError>;

    fn cleanup_workspace(
        &self,
        workspace: &ProviderJobExecutionWorkspace,
    ) -> Result<(), ProviderJobExecutionError>;
}

/// Compatibility data plane for tests and workers that have no artifacts.
/// Any non-empty manifest fails closed.
#[derive(Default)]
pub struct NoArtifactProviderJobDataPlane;

impl ProviderJobDataPlane for NoArtifactProviderJobDataPlane {
    fn prepare_workspace(
        &self,
        assignment: &ProviderJobAssignment,
        _cancellation: &JobCancellation,
    ) -> Result<Option<ProviderJobExecutionWorkspace>, ProviderJobExecutionError> {
        if assignment.job.input_artifacts.is_empty() && assignment.job.expected_outputs.is_empty() {
            Ok(None)
        } else {
            Err(data_plane_error("data_plane_not_configured"))
        }
    }

    fn upload_outputs(
        &self,
        assignment: &ProviderJobAssignment,
        _cancellation: &JobCancellation,
    ) -> Result<Vec<JobArtifact>, ProviderJobExecutionError> {
        if assignment.job.expected_outputs.is_empty() {
            Ok(Vec::new())
        } else {
            Err(data_plane_error("data_plane_not_configured"))
        }
    }

    fn cleanup_workspace(
        &self,
        _workspace: &ProviderJobExecutionWorkspace,
    ) -> Result<(), ProviderJobExecutionError> {
        Ok(())
    }
}

/// HTTP client for job-scoped artifact grants.
///
/// The opaque grant credential stays in this process. Workload containers see
/// only files copied through the runtime backend.
pub struct HttpProviderJobDataPlane {
    control_plane_url: String,
    workspace_root: PathBuf,
    timeout: Duration,
}

impl HttpProviderJobDataPlane {
    pub fn for_control_plane(
        control_plane_url: impl Into<String>,
    ) -> Result<Self, ProviderJobExecutionError> {
        Self::new(
            control_plane_url,
            default_state_dir().join("job-workspaces"),
        )
    }

    pub fn new(
        control_plane_url: impl Into<String>,
        workspace_root: impl Into<PathBuf>,
    ) -> Result<Self, ProviderJobExecutionError> {
        let control_plane_url = control_plane_url.into();
        validate_control_plane_origin(&control_plane_url)?;
        Ok(Self {
            control_plane_url: control_plane_url.trim_end_matches('/').to_string(),
            workspace_root: workspace_root.into(),
            timeout: DATA_PLANE_TIMEOUT,
        })
    }

    fn download_input(
        &self,
        assignment: &ProviderJobAssignment,
        artifact: &JobArtifact,
        grant_url: &JobDataPlaneUrl,
        workspace: &ProviderJobExecutionWorkspace,
        cancellation: &JobCancellation,
    ) -> Result<(), ProviderJobExecutionError> {
        let expected_size = artifact
            .size_bytes
            .filter(|size| *size <= MAX_ARTIFACT_BYTES)
            .ok_or_else(|| data_plane_error("input_size_required"))?;
        let expected_sha256 = artifact
            .sha256
            .as_deref()
            .and_then(normalized_sha256)
            .ok_or_else(|| data_plane_error("input_digest_required"))?;
        let url = self.scoped_url(grant_url)?;
        let request = ureq::get(&url)
            .config()
            .timeout_global(Some(self.timeout))
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        let mut response = request
            .header(
                "Authorization",
                &format!("Bearer {}", assignment.data_plane.credential),
            )
            .call()
            .map_err(|_| data_plane_error("artifact_download_transport"))?;
        if !response.status().is_success() {
            return Err(data_plane_error("artifact_download_rejected"));
        }
        if response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            != Some(expected_size)
        {
            return Err(data_plane_error("artifact_download_size_mismatch"));
        }

        let destination = workspace.inputs_dir.join(&artifact.artifact_id);
        ensure_direct_child(&workspace.inputs_dir, &destination)?;
        let temporary = workspace
            .inputs_dir
            .join(format!(".{}.partial", artifact.artifact_id));
        let mut file = create_private_file_new(&temporary)
            .map_err(|_| data_plane_error("workspace_file_create_failed"))?;
        let transfer = (|| {
            let mut reader = response.body_mut().as_reader();
            let mut buffer = [0_u8; TRANSFER_BUFFER_BYTES];
            let mut transferred = 0_u64;
            let mut digest = Sha256Accumulator::new();
            loop {
                cancellation.ensure_not_cancelled()?;
                let read = reader
                    .read(&mut buffer)
                    .map_err(|_| data_plane_error("artifact_download_read_failed"))?;
                if read == 0 {
                    break;
                }
                transferred = transferred
                    .checked_add(read as u64)
                    .ok_or_else(|| data_plane_error("artifact_download_too_large"))?;
                if transferred > expected_size || transferred > MAX_ARTIFACT_BYTES {
                    return Err(data_plane_error("artifact_download_too_large"));
                }
                digest.update(&buffer[..read]);
                file.write_all(&buffer[..read])
                    .map_err(|_| data_plane_error("workspace_write_failed"))?;
            }
            if transferred != expected_size {
                return Err(data_plane_error("artifact_download_size_mismatch"));
            }
            if digest.finish_hex() != expected_sha256 {
                return Err(data_plane_error("artifact_download_digest_mismatch"));
            }
            file.sync_all()
                .map_err(|_| data_plane_error("workspace_sync_failed"))?;
            drop(file);
            fs::rename(&temporary, &destination)
                .map_err(|_| data_plane_error("workspace_finalize_failed"))?;
            Ok(())
        })();
        if transfer.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        transfer
    }

    fn scoped_url(&self, grant_url: &JobDataPlaneUrl) -> Result<String, ProviderJobExecutionError> {
        if !grant_url.url.starts_with('/')
            || grant_url.url.starts_with("//")
            || grant_url.url.contains('?')
            || grant_url.url.contains('#')
        {
            return Err(data_plane_error("artifact_url_invalid"));
        }
        Ok(join_url(&self.control_plane_url, &grant_url.url))
    }

    fn upload_output(
        &self,
        assignment: &ProviderJobAssignment,
        expected: &JobArtifact,
        grant_url: &JobDataPlaneUrl,
        workspace: &ProviderJobExecutionWorkspace,
        cancellation: &JobCancellation,
    ) -> Result<JobArtifact, ProviderJobExecutionError> {
        cancellation.ensure_not_cancelled()?;
        let maximum_size = expected
            .size_bytes
            .filter(|size| *size <= MAX_ARTIFACT_BYTES)
            .ok_or_else(|| data_plane_error("output_size_required"))?;
        let path = workspace.outputs_dir.join(&expected.artifact_id);
        ensure_direct_child(&workspace.outputs_dir, &path)?;
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| data_plane_error("output_artifact_missing"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(data_plane_error("output_artifact_invalid"));
        }
        restrict_private_file(&path)
            .map_err(|_| data_plane_error("output_artifact_permissions_failed"))?;
        if metadata.len() > maximum_size || metadata.len() > MAX_ARTIFACT_BYTES {
            return Err(data_plane_error("output_artifact_too_large"));
        }
        let mut digest = Sha256Accumulator::new();
        let mut file =
            File::open(&path).map_err(|_| data_plane_error("output_artifact_open_failed"))?;
        let mut buffer = [0_u8; TRANSFER_BUFFER_BYTES];
        let mut size = 0_u64;
        loop {
            cancellation.ensure_not_cancelled()?;
            let read = file
                .read(&mut buffer)
                .map_err(|_| data_plane_error("output_artifact_read_failed"))?;
            if read == 0 {
                break;
            }
            size = size
                .checked_add(read as u64)
                .ok_or_else(|| data_plane_error("output_artifact_too_large"))?;
            if size > maximum_size {
                return Err(data_plane_error("output_artifact_too_large"));
            }
            digest.update(&buffer[..read]);
        }
        let sha256 = format!("sha256:{}", digest.finish_hex());
        file = File::open(&path).map_err(|_| data_plane_error("output_artifact_open_failed"))?;
        let url = self.scoped_url(grant_url)?;
        let request = ureq::put(&url)
            .config()
            .timeout_global(Some(self.timeout))
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        let mut response = request
            .header(
                "Authorization",
                &format!("Bearer {}", assignment.data_plane.credential),
            )
            .header("X-Burd-Content-Sha256", &sha256)
            .header(
                "Content-Type",
                expected
                    .content_type
                    .as_deref()
                    .unwrap_or("application/octet-stream"),
            )
            .send(file)
            .map_err(|_| data_plane_error("artifact_upload_transport"))?;
        if !response.status().is_success() {
            return Err(data_plane_error("artifact_upload_rejected"));
        }
        let receipt = response
            .body_mut()
            .read_json::<JobArtifactUploadResponse>()
            .map_err(|_| data_plane_error("artifact_upload_receipt_invalid"))?;
        if receipt.job_id != assignment.job.job_id
            || receipt.artifact.artifact_id != expected.artifact_id
            || receipt.artifact.object_key != expected.object_key
            || receipt.artifact.sha256.as_deref() != Some(sha256.as_str())
            || receipt.artifact.size_bytes != Some(size)
        {
            return Err(data_plane_error("artifact_upload_receipt_mismatch"));
        }
        Ok(receipt.artifact)
    }
}

impl ProviderJobDataPlane for HttpProviderJobDataPlane {
    fn prepare_workspace(
        &self,
        assignment: &ProviderJobAssignment,
        cancellation: &JobCancellation,
    ) -> Result<Option<ProviderJobExecutionWorkspace>, ProviderJobExecutionError> {
        if assignment.job.input_artifacts.len() > MAX_ARTIFACTS
            || assignment.job.expected_outputs.len() > MAX_ARTIFACTS
        {
            return Err(data_plane_error("artifact_count_invalid"));
        }
        let mut expected_output_bytes = 0_u64;
        for artifact in &assignment.job.expected_outputs {
            let maximum = artifact
                .size_bytes
                .filter(|size| *size <= MAX_ARTIFACT_BYTES)
                .ok_or_else(|| data_plane_error("output_size_required"))?;
            expected_output_bytes = expected_output_bytes
                .checked_add(maximum)
                .filter(|total| *total <= MAX_ARTIFACT_BYTES)
                .ok_or_else(|| data_plane_error("output_artifact_total_too_large"))?;
        }
        if assignment.job.input_artifacts.is_empty() && assignment.job.expected_outputs.is_empty() {
            return Ok(None);
        }
        cancellation.ensure_not_cancelled()?;
        create_private_directory_all(&self.workspace_root)
            .map_err(|_| data_plane_error("workspace_root_create_failed"))?;
        let token =
            random_token("job_workspace").map_err(|_| data_plane_error("workspace_id_failed"))?;
        let root = self.workspace_root.join(token);
        let workspace = ProviderJobExecutionWorkspace {
            inputs_dir: root.join("inputs"),
            outputs_dir: root.join("outputs"),
            root,
        };
        create_private_directory_all(&workspace.inputs_dir)
            .and_then(|_| create_private_directory_all(&workspace.outputs_dir))
            .map_err(|_| data_plane_error("workspace_create_failed"))?;

        let urls = urls_by_artifact(&assignment.data_plane.download_urls)?;
        let prepared = assignment
            .job
            .input_artifacts
            .iter()
            .try_for_each(|artifact| {
                let url = urls
                    .get(artifact.artifact_id.as_str())
                    .ok_or_else(|| data_plane_error("artifact_download_grant_missing"))?;
                self.download_input(assignment, artifact, url, &workspace, cancellation)
            });
        if let Err(error) = prepared {
            let _ = self.cleanup_workspace(&workspace);
            return Err(error);
        }
        Ok(Some(workspace))
    }

    fn upload_outputs(
        &self,
        assignment: &ProviderJobAssignment,
        cancellation: &JobCancellation,
    ) -> Result<Vec<JobArtifact>, ProviderJobExecutionError> {
        if assignment.job.expected_outputs.is_empty() {
            return Ok(Vec::new());
        }
        let workspace = assignment
            .workspace
            .as_ref()
            .ok_or_else(|| data_plane_error("workspace_missing"))?;
        reject_undeclared_outputs(&workspace.outputs_dir, &assignment.job.expected_outputs)?;
        let urls = urls_by_artifact(&assignment.data_plane.upload_urls)?;
        assignment
            .job
            .expected_outputs
            .iter()
            .map(|artifact| {
                let url = urls
                    .get(artifact.artifact_id.as_str())
                    .ok_or_else(|| data_plane_error("artifact_upload_grant_missing"))?;
                self.upload_output(assignment, artifact, url, workspace, cancellation)
            })
            .collect()
    }

    fn cleanup_workspace(
        &self,
        workspace: &ProviderJobExecutionWorkspace,
    ) -> Result<(), ProviderJobExecutionError> {
        cleanup_workspace_under(&self.workspace_root, &workspace.root)
    }
}

fn urls_by_artifact(
    urls: &[JobDataPlaneUrl],
) -> Result<HashMap<&str, &JobDataPlaneUrl>, ProviderJobExecutionError> {
    let mut mapped = HashMap::with_capacity(urls.len());
    for url in urls {
        if mapped.insert(url.artifact_id.as_str(), url).is_some() {
            return Err(data_plane_error("artifact_grant_ambiguous"));
        }
    }
    Ok(mapped)
}

fn normalized_sha256(value: &str) -> Option<String> {
    let digest = value.strip_prefix("sha256:")?;
    (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| digest.to_ascii_lowercase())
}

fn ensure_direct_child(parent: &Path, child: &Path) -> Result<(), ProviderJobExecutionError> {
    if child.parent() != Some(parent)
        || child
            .file_name()
            .is_none_or(|name| name.is_empty() || name.to_string_lossy().contains(['/', '\\']))
    {
        return Err(data_plane_error("artifact_path_invalid"));
    }
    Ok(())
}

fn reject_undeclared_outputs(
    directory: &Path,
    expected: &[JobArtifact],
) -> Result<(), ProviderJobExecutionError> {
    let expected = expected
        .iter()
        .map(|artifact| artifact.artifact_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let entries = fs::read_dir(directory).map_err(|_| data_plane_error("output_scan_failed"))?;
    for entry in entries {
        let entry = entry.map_err(|_| data_plane_error("output_scan_failed"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(data_plane_error("undeclared_output_artifact"));
        };
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|_| data_plane_error("output_scan_failed"))?;
        if !expected.contains(name) || !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(data_plane_error("undeclared_output_artifact"));
        }
    }
    Ok(())
}

fn cleanup_workspace_under(root: &Path, workspace: &Path) -> Result<(), ProviderJobExecutionError> {
    if !workspace.exists() {
        return Ok(());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| data_plane_error("workspace_cleanup_scope_invalid"))?;
    let canonical_workspace = workspace
        .canonicalize()
        .map_err(|_| data_plane_error("workspace_cleanup_scope_invalid"))?;
    if canonical_workspace.parent() != Some(canonical_root.as_path())
        || canonical_workspace
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !name.starts_with("job_workspace_"))
    {
        return Err(data_plane_error("workspace_cleanup_scope_invalid"));
    }
    fs::remove_dir_all(&canonical_workspace)
        .map_err(|_| data_plane_error("workspace_cleanup_failed"))
}

fn validate_control_plane_origin(origin: &str) -> Result<(), ProviderJobExecutionError> {
    let uri = origin
        .parse::<ureq::http::Uri>()
        .map_err(|_| data_plane_error("data_plane_origin_invalid"))?;
    let scheme = uri.scheme_str();
    let host = uri.host().unwrap_or_default();
    let loopback_http =
        scheme == Some("http") && matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]");
    if (scheme != Some("https") && !loopback_http)
        || uri.authority().is_none()
        || uri.query().is_some()
        || origin.contains('@')
    {
        return Err(data_plane_error("data_plane_origin_invalid"));
    }
    Ok(())
}

fn data_plane_error(code: &'static str) -> ProviderJobExecutionError {
    ProviderJobExecutionError::new(code, "provider job artifact data-plane operation failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_require_https_except_for_loopback_tests() {
        assert!(HttpProviderJobDataPlane::new("https://control.burd.example", "work").is_ok());
        assert!(HttpProviderJobDataPlane::new("http://127.0.0.1:8080", "work").is_ok());
        assert!(HttpProviderJobDataPlane::new("http://control.burd.example", "work").is_err());
        assert!(
            HttpProviderJobDataPlane::new("https://user@control.burd.example", "work").is_err()
        );
    }

    #[test]
    fn cleanup_refuses_paths_outside_the_workspace_root() {
        let root =
            std::env::temp_dir().join(format!("burd-data-plane-root-{}", std::process::id()));
        let outside =
            std::env::temp_dir().join(format!("burd-data-plane-outside-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        assert_eq!(
            cleanup_workspace_under(&root, &outside).unwrap_err().code(),
            "workspace_cleanup_scope_invalid"
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn output_scan_rejects_undeclared_files() {
        let root = std::env::temp_dir().join(format!(
            "burd-data-plane-output-scan-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("unexpected"), b"x").unwrap();
        assert_eq!(
            reject_undeclared_outputs(&root, &[]).unwrap_err().code(),
            "undeclared_output_artifact"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
