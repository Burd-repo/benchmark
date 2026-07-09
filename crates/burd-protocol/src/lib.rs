pub mod challenge;
pub mod enrollment;
pub mod evidence;
pub mod identity;
pub mod remote_session;
pub mod report;
pub mod session;
pub mod signature;
pub mod telemetry;

pub use challenge::{
    Challenge, ChallengePolicy, ChallengeResponse, ChallengeRunOutput, ChallengeVerification,
    RequiredTest, challenge_expired, challenge_response_message,
    challenge_response_message_with_fingerprint, load_latest_challenge_output, mock_challenge,
    save_latest_challenge_output, verify_challenge_response,
};
pub use enrollment::{
    DeviceCredentialResponse, DeviceRecord, DeviceRevocationResponse, ENROLLMENT_PROOF_DOMAIN,
    EnrollmentProofClaims, EnrollmentProofRequest, EnrollmentProofResponse,
    IssueEnrollmentTokenResponse, KEY_ROTATION_PROOF_DOMAIN, KeyRotationProofClaims,
    KeyRotationProofRequest, KeyRotationProofResponse, RemoteEnrollmentState,
    RemoteEnrollmentStatus, StartEnrollmentRequest, StartEnrollmentResponse,
    StartKeyRotationRequest, StartKeyRotationResponse, enrollment_proof_message,
    key_rotation_proof_message, load_remote_enrollment, remote_enrollment_path,
    save_remote_enrollment, show_remote_enrollment, update_remote_credential,
};
pub use evidence::{
    CHALLENGE_TTL_SECONDS, EvidenceFreshness, FULL_REPORT_TTL_SECONDS, SIGNED_REPORT_TTL_SECONDS,
    evidence_freshness, evidence_freshness_at, evidence_freshness_from_window,
    evidence_freshness_from_window_at,
};
pub use identity::{
    AgentConfig, AgentIdentityPublic, AgentStatePaths, ApiTokenStatus, IdentityInitResult,
    IdentityMigrationResult, IdentityStatus, PrivateKeyFile, agent_state_paths, create_api_token,
    default_config_path, default_state_dir, init_identity, load_identity, load_private_key,
    migrate_identity, redacted_config_value, rotate_api_token, rotate_identity_key,
    show_api_token_status, show_identity, verify_api_token,
};
pub use remote_session::{
    ClientControlMessage, HeartbeatPayload, HeartbeatReceipt, RemoteSessionRecord,
    RemoteSessionResume, RemoteSessionRevocationResponse, RemoteSessionState,
    RemoteSessionStateStatus, RemoteSessionStatus, ServerControlMessage, StartRemoteSessionRequest,
    StartRemoteSessionResponse, clear_remote_session, load_remote_session, new_resume_token,
    remote_session_path, save_remote_session, show_remote_session, update_remote_session_sequence,
    update_remote_telemetry_sequence,
};
pub use report::{FullReport, ReportSignature, SignedReport, VerifyReportResult};
pub use session::{
    ProviderHeartbeatSummary, ProviderSession, ProviderSessionMode, ProviderSessionStatus,
    ProviderSessionStatusReport, active_provider_session, heartbeat_summary_from_session,
    load_provider_session, new_provider_session_id, provider_session_path, save_provider_session,
    session_status_from_session,
};
pub use signature::{
    KEY_ALGORITHM, KeyMaterial, canonical_json, canonical_json_value, generate_keypair,
    hash_canonical, placeholder_signature, random_token, sha256_hex, sign_message,
    validate_public_key, verify_message,
};
pub use telemetry::{
    GpuProcessTelemetry, GpuTelemetrySample, LatestTelemetryResponse, SignedTelemetryBatch,
    TELEMETRY_CANONICALIZATION_VERSION, TELEMETRY_SCHEMA_VERSION, TELEMETRY_SIGNATURE_DOMAIN,
    TelemetryBatchPayload, TelemetryBatchReceipt, telemetry_batch_hash,
    telemetry_batch_signature_message,
};
