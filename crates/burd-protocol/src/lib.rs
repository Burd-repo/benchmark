pub mod challenge;
pub mod identity;
pub mod report;
pub mod signature;

pub use challenge::{
    Challenge, ChallengePolicy, ChallengeResponse, ChallengeVerification, RequiredTest,
    challenge_expired, challenge_response_message, mock_challenge, verify_challenge_response,
};
pub use identity::{
    AgentConfig, AgentIdentityPublic, ApiTokenStatus, IdentityInitResult, IdentityStatus,
    PrivateKeyFile, create_api_token, default_config_path, default_state_dir, init_identity,
    load_identity, load_private_key, redacted_config_value, rotate_api_token, rotate_identity_key,
    show_api_token_status, show_identity, verify_api_token,
};
pub use report::{FullReport, ReportSignature, SignedReport, VerifyReportResult};
pub use signature::{
    KEY_ALGORITHM, canonical_json, canonical_json_value, hash_canonical, placeholder_signature,
    sha256_hex, sign_message, verify_message,
};
