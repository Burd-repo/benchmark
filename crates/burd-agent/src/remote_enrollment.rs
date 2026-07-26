use burd_bench::build_registration_payload;
use burd_protocol::{
    DeviceCredentialResponse, EnrollmentProofRequest, EnrollmentProofResponse,
    RemoteEnrollmentStatus, StartEnrollmentRequest, StartEnrollmentResponse, clear_remote_session,
    enrollment_proof_message, load_identity, load_private_key, load_remote_enrollment,
    save_remote_enrollment, sign_message, update_remote_credential,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ControlPlaneRequestError {
    LocalState(String),
    Transport(String),
    Rejected {
        status: u16,
        code: String,
        message: String,
    },
    Contract(String),
}

impl ControlPlaneRequestError {
    pub(crate) fn is_code(&self, expected: &str) -> bool {
        matches!(self, Self::Rejected { code, .. } if code == expected)
    }
}

impl fmt::Display for ControlPlaneRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalState(message) | Self::Transport(message) | Self::Contract(message) => {
                formatter.write_str(message)
            }
            Self::Rejected { code, message, .. } => {
                write!(formatter, "control plane {code}: {message}")
            }
        }
    }
}

pub fn enroll(
    control_plane_url: &str,
    enrollment_token: String,
    agent_version: &str,
) -> Result<RemoteEnrollmentStatus, String> {
    let identity = load_identity()?;
    let private_key = load_private_key(&identity)?;
    let registration = build_registration_payload(agent_version);
    let start_request = StartEnrollmentRequest {
        enrollment_token,
        public_key: identity.public_key.clone(),
        key_algorithm: identity.key_algorithm.clone(),
        local_provider_id: Some(identity.provider_id.clone()),
        machine_id: identity.machine_id.clone(),
        registration_payload: serde_json::to_value(&registration)
            .map_err(|error| format!("failed to serialize registration payload: {error}"))?,
        hardware_fingerprint: registration.hardware_fingerprint,
        agent_version: registration.agent_version,
        benchmark_version: registration.benchmark_version,
    };
    let started: StartEnrollmentResponse = post_json(
        &join_url(control_plane_url, "/v1/enrollments"),
        &start_request,
        None,
    )?;
    let message = enrollment_proof_message(
        &started.enrollment_id,
        &started.provider_id,
        &start_request.machine_id,
        &started.nonce,
        &start_request.public_key,
        &start_request.hardware_fingerprint,
        &started.expires_at,
    )?;
    let signature = sign_message(&private_key.secret_key_base64, message.as_bytes())?;
    let proof = EnrollmentProofRequest {
        nonce: started.nonce,
        signature,
        public_key: start_request.public_key,
        hardware_fingerprint: start_request.hardware_fingerprint,
    };
    let response: EnrollmentProofResponse = post_json(
        &join_url(
            control_plane_url,
            &format!("/v1/enrollments/{}/proof", started.enrollment_id),
        ),
        &proof,
        None,
    )?;
    clear_remote_session()?;
    save_remote_enrollment(control_plane_url, &response)
}

pub fn refresh_credential() -> Result<RemoteEnrollmentStatus, String> {
    refresh_credential_checked().map_err(|error| error.to_string())
}

pub(crate) fn refresh_credential_checked()
-> Result<RemoteEnrollmentStatus, ControlPlaneRequestError> {
    let state = load_remote_enrollment().map_err(ControlPlaneRequestError::LocalState)?;
    let response: DeviceCredentialResponse = post_json_checked(
        &join_url(
            &state.control_plane_url,
            &format!("/v1/devices/{}/credentials", state.device_id),
        ),
        &serde_json::json!({}),
        Some(&state.credential),
    )?;
    update_remote_credential(&response).map_err(ControlPlaneRequestError::LocalState)
}

pub(crate) fn post_json<TRequest, TResponse>(
    url: &str,
    payload: &TRequest,
    bearer: Option<&str>,
) -> Result<TResponse, String>
where
    TRequest: Serialize,
    TResponse: DeserializeOwned,
{
    post_json_checked(url, payload, bearer).map_err(|error| error.to_string())
}

pub(crate) fn post_json_checked<TRequest, TResponse>(
    url: &str,
    payload: &TRequest,
    bearer: Option<&str>,
) -> Result<TResponse, ControlPlaneRequestError>
where
    TRequest: Serialize,
    TResponse: DeserializeOwned,
{
    let request = ureq::post(url)
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build();
    let request = if let Some(token) = bearer {
        request.header("Authorization", &format!("Bearer {token}"))
    } else {
        request
    };
    let mut response = request.send_json(payload).map_err(|error| {
        ControlPlaneRequestError::Transport(format!("control plane request failed: {error}"))
    })?;
    let status = response.status();
    let value = response.body_mut().read_json::<serde_json::Value>();
    if !status.is_success() {
        let value = value.unwrap_or(serde_json::Value::Null);
        let code = value["error"]["code"]
            .as_str()
            .unwrap_or("remote_error")
            .to_string();
        let message = value["error"]["message"]
            .as_str()
            .unwrap_or("control plane rejected request")
            .to_string();
        return Err(ControlPlaneRequestError::Rejected {
            status: status.as_u16(),
            code,
            message,
        });
    }
    let value = value.map_err(|error| {
        ControlPlaneRequestError::Contract(format!("control plane returned invalid JSON: {error}"))
    })?;
    serde_json::from_value(value).map_err(|error| {
        ControlPlaneRequestError::Contract(format!(
            "invalid control plane response contract: {error}"
        ))
    })
}

pub(crate) fn join_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_plane_urls_are_joined_without_double_slash() {
        assert_eq!(
            join_url("https://api.burd.cloud/", "/v1/enrollments"),
            "https://api.burd.cloud/v1/enrollments"
        );
    }

    #[test]
    fn rejected_request_errors_preserve_status_and_code() {
        let error = ControlPlaneRequestError::Rejected {
            status: 403,
            code: "revoked".to_string(),
            message: "device has been revoked".to_string(),
        };
        assert!(error.is_code("revoked"));
        assert!(!error.is_code("expired"));
        assert_eq!(
            error.to_string(),
            "control plane revoked: device has been revoked"
        );
    }
}
