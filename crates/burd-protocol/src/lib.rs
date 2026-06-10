pub mod challenge;
pub mod identity;
pub mod report;
pub mod signature;

pub use challenge::{
    Challenge, ChallengePolicy, ChallengeResponse, ChallengeRunOutput, ChallengeVerification,
    RequiredTest, challenge_expired, challenge_response_message, load_latest_challenge_output,
    mock_challenge, save_latest_challenge_output, verify_challenge_response,
};
pub use identity::{
    AgentConfig, AgentIdentityPublic, AgentStatePaths, ApiTokenStatus, IdentityInitResult,
    IdentityMigrationResult, IdentityStatus, PrivateKeyFile, agent_state_paths, create_api_token,
    default_config_path, default_state_dir, init_identity, load_identity, load_private_key,
    migrate_identity, redacted_config_value, rotate_api_token, rotate_identity_key,
    show_api_token_status, show_identity, verify_api_token,
};
pub use report::{FullReport, ReportSignature, SignedReport, VerifyReportResult};
pub use signature::{
    KEY_ALGORITHM, canonical_json, canonical_json_value, hash_canonical, placeholder_signature,
    sha256_hex, sign_message, verify_message,
};
