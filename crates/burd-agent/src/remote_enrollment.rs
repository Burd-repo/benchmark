use burd_bench::build_registration_payload;
use burd_protocol::{
    DeviceCredentialResponse, EnrollmentProofRequest, EnrollmentProofResponse,
    RemoteEnrollmentStatus, StartEnrollmentRequest, StartEnrollmentResponse,
    enrollment_proof_message, load_identity, load_private_key, load_remote_enrollment,
    save_remote_enrollment, sign_message, update_remote_credential,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

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
    save_remote_enrollment(control_plane_url, &response)
}

pub fn refresh_credential() -> Result<RemoteEnrollmentStatus, String> {
    let state = load_remote_enrollment()?;
    let response: DeviceCredentialResponse = post_json(
        &join_url(
            &state.control_plane_url,
            &format!("/v1/devices/{}/credentials", state.device_id),
        ),
        &serde_json::json!({}),
        Some(&state.credential),
    )?;
    update_remote_credential(&response)
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
    let mut response = request
        .send_json(payload)
        .map_err(|error| format!("control plane request failed: {error}"))?;
    let status = response.status();
    let value: serde_json::Value = response
        .body_mut()
        .read_json()
        .map_err(|error| format!("control plane returned invalid JSON: {error}"))?;
    if !status.is_success() {
        let code = value["error"]["code"].as_str().unwrap_or("remote_error");
        let message = value["error"]["message"]
            .as_str()
            .unwrap_or("control plane rejected request");
        return Err(format!("control plane {code}: {message}"));
    }
    serde_json::from_value(value)
        .map_err(|error| format!("invalid control plane response contract: {error}"))
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
}
