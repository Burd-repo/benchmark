use crate::raw::build_raw_data_from_provider;
use crate::registration::build_registration_payload_from;
use crate::test_fixtures;
use crate::verification::verify_provider_from_reports;
use crate::{
    append_signed_report_history, export_history, load_history_list, load_latest_history,
    load_latest_signed_report, save_latest_signed_report, verify_signed_report,
};
use burd_protocol::{
    Challenge, ChallengePolicy, ChallengeResponse, RequiredTest, SignedReport,
    challenge_response_message, create_api_token, default_config_path, default_state_dir,
    load_identity, load_private_key, redacted_config_value, sha256_hex, show_api_token_status,
    sign_message, verify_api_token, verify_challenge_response,
};
use chrono::{Duration, Utc};
use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const TEST_AGENT_VERSION: &str = "0.1.0-test";

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[test]
fn signed_report_contract_uses_temp_identity_and_hides_secrets() {
    let _guard = env_lock();
    let env = TestEnv::new("signed-report");
    env.assert_active();
    env.install_identity();
    let api_token = create_api_token().unwrap().token.unwrap();

    let signed = test_fixtures::signed_report(None).unwrap();
    save_latest_signed_report(&signed).unwrap();
    let verification = verify_signed_report(&signed);

    assert!(!signed.report_hash.is_empty());
    assert!(!signed.signature.is_empty());
    assert!(!signed.public_key.is_empty());
    assert_eq!(signed.key_algorithm, "ed25519");
    assert!(!signed.signed_at.is_empty());
    assert!(signed.signature_valid_locally);
    assert!(verification.signature_valid);
    assert!(verification.errors.is_empty());
    assert_no_secret_fields(&serde_json::to_value(&signed).unwrap(), Some(&api_token));
    assert_no_secret_values(&serde_json::to_value(&signed).unwrap(), Some(&api_token));
}

#[test]
fn contract_report_fixtures_are_deterministic() {
    let _guard = env_lock();
    let env = TestEnv::new("deterministic-fixtures");
    env.assert_active();
    env.install_identity();

    assert_eq!(
        serde_json::to_value(test_fixtures::system_report()).unwrap(),
        serde_json::to_value(test_fixtures::system_report()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(test_fixtures::fit_report()).unwrap(),
        serde_json::to_value(test_fixtures::fit_report()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(test_fixtures::score_report()).unwrap(),
        serde_json::to_value(test_fixtures::score_report()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(test_fixtures::signed_report(None).unwrap()).unwrap(),
        serde_json::to_value(test_fixtures::signed_report(None).unwrap()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(test_fixtures::provider_details()).unwrap(),
        serde_json::to_value(test_fixtures::provider_details()).unwrap()
    );
}

#[test]
fn sanitized_json_contract_snapshots_are_stable() {
    let _guard = env_lock();
    let env = TestEnv::new("json-contract-snapshots");
    env.assert_active();
    env.install_identity();

    let signed = test_fixtures::signed_report(None).unwrap();
    save_latest_signed_report(&signed).unwrap();
    let provider = test_fixtures::provider_details();
    let verification = test_fixtures::provider_verification();
    let identity = load_identity().unwrap();
    let registration = build_registration_payload_from(
        TEST_AGENT_VERSION,
        &provider,
        Some(&identity),
        Some(&signed),
        &verification,
        test_fixtures::FIXTURE_TIMESTAMP.to_string(),
    );
    let raw = build_raw_data_from_provider(&provider, &verification);

    for (name, value) in [
        (
            "provider-details.json",
            serde_json::to_value(&provider).unwrap(),
        ),
        ("raw-data.json", serde_json::to_value(&raw).unwrap()),
        (
            "registration-payload.json",
            serde_json::to_value(&registration).unwrap(),
        ),
    ] {
        assert_contract_snapshot(name, value, &env);
    }
}

#[test]
fn challenge_local_contract_validates_success_expiry_required_tests_and_nonce() {
    let _guard = env_lock();
    let env = TestEnv::new("challenge");
    env.assert_active();
    env.install_identity();
    let api_token = create_api_token().unwrap().token.unwrap();

    let challenge = light_challenge();
    let signed_report = signed_report_for_challenge(&challenge);
    let (response, verification) = response_for_signed_report(&challenge, signed_report.clone());

    assert!(verification.valid, "{:?}", verification.errors);
    assert!(!verification.expired);
    assert_eq!(response.status, "passed");
    assert_eq!(response.challenge_id, challenge.challenge_id);
    assert_eq!(response.nonce, challenge.nonce);
    assert!(!response.provider_id.is_empty());
    assert!(!response.machine_id.is_empty());
    assert!(!response.report_hash.is_empty());
    assert!(!response.signature.is_empty());
    assert!(response.signed_report.is_some());
    assert!(!response.completed_at.is_empty());
    assert_no_secret_values(&serde_json::to_value(&response).unwrap(), Some(&api_token));

    let mut expired = light_challenge();
    expired.expires_at = (Utc::now() - Duration::seconds(1)).to_rfc3339();
    let (_response, verification) = response_for_signed_report(&expired, signed_report.clone());
    assert!(verification.expired);
    assert!(
        verification
            .errors
            .iter()
            .any(|error| error == "challenge expired")
    );

    let strict_challenge = strict_challenge();
    let (_response, verification) =
        response_for_signed_report(&strict_challenge, signed_report.clone());
    assert!(!verification.valid);
    assert!(
        verification
            .errors
            .iter()
            .any(|error| error.contains("required test missing")
                || error.contains("policy requires"))
    );

    let nonce_challenge = challenge;
    let (mut response, _verification) = response_for_signed_report(&nonce_challenge, signed_report);
    response.nonce = "wrong-nonce".to_string();
    let verification = verify_challenge_response(&nonce_challenge, &response);
    assert!(!verification.valid);
    assert!(
        verification
            .errors
            .iter()
            .any(|error| error == "nonce mismatch")
    );
}

#[test]
fn registration_payload_contract_uses_latest_signed_report_without_secrets() {
    let _guard = env_lock();
    let env = TestEnv::new("registration");
    env.assert_active();
    env.install_identity();
    let api_token = create_api_token().unwrap().token.unwrap();
    let signed = test_fixtures::signed_report(None).unwrap();
    save_latest_signed_report(&signed).unwrap();

    let provider = test_fixtures::provider_details();
    let verification = test_fixtures::provider_verification();
    let identity = load_identity().unwrap();
    let payload = build_registration_payload_from(
        TEST_AGENT_VERSION,
        &provider,
        Some(&identity),
        Some(&signed),
        &verification,
        test_fixtures::FIXTURE_TIMESTAMP.to_string(),
    );
    let value = serde_json::to_value(&payload).unwrap();

    assert_eq!(payload.provider_id, signed.provider_id);
    assert_eq!(payload.machine_id, signed.machine_id);
    assert_eq!(
        payload.public_key.as_deref(),
        Some(signed.public_key.as_str())
    );
    assert_eq!(payload.agent_version, TEST_AGENT_VERSION);
    assert!(!payload.benchmark_version.is_empty());
    assert!(value.get("provider_details").is_some());
    assert_eq!(
        payload.latest_signed_report_hash.as_deref(),
        Some(signed.report_hash.as_str())
    );
    assert!(payload.latest_score.is_some());
    assert!(payload.latest_tier.is_some());
    assert!(value.get("capabilities").is_some());
    assert!(value.get("pricing").is_some());
    assert!(value.get("verification").is_some());
    assert!(!payload.created_at.is_empty());
    assert!(!payload.secrets_included);
    assert_no_secret_fields(&value, Some(&api_token));
    assert_no_secret_values(&value, Some(&api_token));
}

#[test]
fn benchmark_history_contract_stores_summaries_and_exports_without_secrets() {
    let _guard = env_lock();
    let env = TestEnv::new("history");
    env.assert_active();
    env.install_identity();
    let api_token = create_api_token().unwrap().token.unwrap();

    let initial = load_history_list().unwrap();
    assert_eq!(initial.entries_total, 0);
    assert!(initial.entries.is_empty());

    let challenge = light_challenge();
    let signed = signed_report_for_challenge(&challenge);
    let entry = append_signed_report_history(&signed).unwrap();
    assert!(!entry.timestamp.is_empty());
    assert_eq!(
        entry.provider_id.as_deref(),
        Some(signed.provider_id.as_str())
    );
    assert_eq!(
        entry.machine_id.as_deref(),
        Some(signed.machine_id.as_str())
    );
    assert!(!entry.benchmark_profile.is_empty());
    assert!(entry.score >= 0.0);
    assert!(!entry.tier.is_empty());
    assert_eq!(entry.report_hash, signed.report_hash);
    assert!(entry.signed);
    assert_eq!(
        entry.challenge_id.as_deref(),
        Some(challenge.challenge_id.as_str())
    );
    let _warnings: Vec<String> = entry.warnings.clone();

    let list = load_history_list().unwrap();
    assert_eq!(list.entries_total, 1);
    let latest = load_latest_history().unwrap();
    assert_eq!(latest.entries_total, 1);
    assert!(latest.latest.is_some());

    let export_path = env.path("exports").join("history.json");
    let export = export_history(&export_path).unwrap();
    assert_eq!(export.entries_exported, 1);
    assert!(export_path.exists());

    for value in [
        serde_json::to_value(&entry).unwrap(),
        serde_json::to_value(&list).unwrap(),
        serde_json::from_str::<Value>(&fs::read_to_string(&export_path).unwrap()).unwrap(),
    ] {
        assert_no_secret_fields(&value, Some(&api_token));
        assert_no_secret_values(&value, Some(&api_token));
    }
}

#[test]
fn api_token_status_and_raw_redaction_hide_token_values() {
    let _guard = env_lock();
    let env = TestEnv::new("api-token-redaction");
    env.assert_active();
    env.install_identity();
    let created = create_api_token().unwrap();
    let token = created.token.unwrap();
    let token_hash = sha256_hex(token.as_bytes());

    let status = show_api_token_status().unwrap();
    assert!(status.api_auth_enabled);
    assert!(status.token_configured);
    assert!(status.token_hash_preview.is_some());
    assert!(status.token.is_none());
    assert!(verify_api_token(&token).unwrap());
    assert!(!verify_api_token("invalid-token").unwrap());

    let config = redacted_config_value().unwrap();
    assert_eq!(config["private_key_path"], "[redacted]");
    assert_eq!(config["api_token_hash"], "[redacted]");

    let provider = test_fixtures::provider_details();
    let verification = test_fixtures::provider_verification();
    let raw = build_raw_data_from_provider(&provider, &verification);
    assert!(raw.redacted);
    assert!(raw.redacted_fields.iter().any(|field| field == "api_token"));
    assert!(
        raw.redacted_fields
            .iter()
            .any(|field| field == "api_token_hash")
    );
    assert!(
        raw.redacted_fields
            .iter()
            .any(|field| field == "secret_key_base64")
    );
    assert_eq!(
        raw.config_redacted
            .as_ref()
            .and_then(|value| value.get("private_key_path"))
            .and_then(|value| value.as_str()),
        Some("[redacted]")
    );
    assert_eq!(
        raw.config_redacted
            .as_ref()
            .and_then(|value| value.get("api_token_hash"))
            .and_then(|value| value.as_str()),
        Some("[redacted]")
    );

    let raw_value = serde_json::to_value(&raw).unwrap();
    assert_no_secret_values(&raw_value, Some(&token));
    let raw_json = serde_json::to_string(&raw_value).unwrap();
    assert!(!raw_json.contains(&token_hash));
}

#[test]
fn provider_readiness_flow_contract_distinguishes_local_states() {
    let _guard = env_lock();
    let env = TestEnv::new("provider-readiness");
    env.assert_active();

    let fresh = classify_local_readiness(TEST_AGENT_VERSION);
    assert_eq!(fresh.status, LocalReadinessStatus::Uninitialized);
    assert!(fresh.messages.iter().any(|item| item == "identity missing"));
    assert!(
        fresh
            .messages
            .iter()
            .any(|item| item == "signed report missing")
    );

    env.install_identity();
    let initialized = classify_local_readiness(TEST_AGENT_VERSION);
    assert_eq!(initialized.status, LocalReadinessStatus::NotVerified);
    assert!(
        initialized
            .messages
            .iter()
            .any(|item| item == "signed report missing")
    );
    assert!(
        initialized
            .messages
            .iter()
            .any(|item| item == "challenge pending")
    );

    let signed = test_fixtures::signed_report(None).unwrap();
    save_latest_signed_report(&signed).unwrap();
    append_signed_report_history(&signed).unwrap();
    let provider = test_fixtures::provider_details();
    let verification = test_fixtures::provider_verification();
    let identity = load_identity().unwrap();
    let payload = build_registration_payload_from(
        TEST_AGENT_VERSION,
        &provider,
        Some(&identity),
        Some(&signed),
        &verification,
        test_fixtures::FIXTURE_TIMESTAMP.to_string(),
    );
    let latest = load_latest_history().unwrap();
    let raw = build_raw_data_from_provider(&provider, &verification);
    let ready = classify_local_readiness(TEST_AGENT_VERSION);
    assert_eq!(ready.status, LocalReadinessStatus::ReadyLocally);
    assert!(ready.messages.iter().any(|item| item == "ready locally"));
    assert!(
        ready
            .messages
            .iter()
            .any(|item| item == "challenge pending")
    );
    assert_eq!(
        payload.latest_signed_report_hash.as_deref(),
        Some(signed.report_hash.as_str())
    );
    assert!(latest.latest.is_some());
    assert!(raw.redacted);

    let mut tampered = load_latest_signed_report().unwrap();
    tampered.signature = "bad-signature".to_string();
    save_latest_signed_report(&tampered).unwrap();
    let failed = classify_local_readiness(TEST_AGENT_VERSION);
    assert_eq!(failed.status, LocalReadinessStatus::Failed);
    assert!(
        failed
            .messages
            .iter()
            .any(|item| item == "signed report failed")
    );
}

#[test]
#[ignore = "slow integration test: exercises real local hardware detection"]
fn real_hardware_detection_integration_is_available() {
    let report = burd_hardware::detect_system_report(TEST_AGENT_VERSION);

    assert!(!report.os.is_empty());
    assert!(!report.architecture.is_empty());
    assert!(!report.cpu.trim().is_empty());
    assert!(report.cpu_cores > 0);
}

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn light_challenge() -> Challenge {
    let mut challenge = strict_challenge();
    challenge.required_tests = vec![
        RequiredTest {
            name: "system".to_string(),
            required: true,
        },
        RequiredTest {
            name: "fit".to_string(),
            required: true,
        },
    ];
    challenge.policy = ChallengePolicy {
        require_signed_report: true,
        require_llm_benchmark: false,
        require_stability: false,
        require_network: false,
        require_disk: false,
    };
    challenge
}

fn strict_challenge() -> Challenge {
    Challenge {
        challenge_id: "challenge-contract".to_string(),
        nonce: "nonce-contract".to_string(),
        benchmark_profile: "profile_8gb".to_string(),
        required_tests: vec![
            RequiredTest {
                name: "system".to_string(),
                required: true,
            },
            RequiredTest {
                name: "fit".to_string(),
                required: true,
            },
            RequiredTest {
                name: "llm_benchmark".to_string(),
                required: true,
            },
            RequiredTest {
                name: "stability".to_string(),
                required: true,
            },
            RequiredTest {
                name: "network".to_string(),
                required: false,
            },
            RequiredTest {
                name: "disk".to_string(),
                required: false,
            },
        ],
        issued_at: test_fixtures::FIXTURE_TIMESTAMP.to_string(),
        expires_at: "2099-01-01T00:00:00Z".to_string(),
        backend_url: Some("https://api.burd.cloud".to_string()),
        min_agent_version: "0.1.0".to_string(),
        min_benchmark_version: burd_hardware::BENCHMARK_VERSION.to_string(),
        policy: ChallengePolicy {
            require_signed_report: true,
            require_llm_benchmark: true,
            require_stability: true,
            require_network: true,
            require_disk: true,
        },
    }
}

fn signed_report_for_challenge(challenge: &Challenge) -> SignedReport {
    test_fixtures::signed_report(Some(challenge.clone())).unwrap()
}

fn response_for_signed_report(
    challenge: &Challenge,
    signed_report: SignedReport,
) -> (ChallengeResponse, burd_protocol::ChallengeVerification) {
    let config = load_identity().unwrap();
    let private_key = load_private_key(&config).unwrap();
    let message = challenge_response_message(
        &challenge.challenge_id,
        &challenge.nonce,
        &config.provider_id,
        &config.machine_id,
        &signed_report.report_hash,
    )
    .unwrap();
    let signature = sign_message(&private_key.secret_key_base64, message.as_bytes()).unwrap();
    let mut response = ChallengeResponse {
        challenge_id: challenge.challenge_id.clone(),
        nonce: challenge.nonce.clone(),
        provider_id: config.provider_id,
        machine_id: config.machine_id,
        report_hash: signed_report.report_hash.clone(),
        signed_report: Some(signed_report.clone()),
        signature,
        public_key: signed_report.public_key.clone(),
        completed_at: test_fixtures::FIXTURE_TIMESTAMP.to_string(),
        status: "partial".to_string(),
        failed_requirements: Vec::new(),
        verification_result: None,
    };
    let verification = verify_challenge_response(challenge, &response);
    response.status = if verification.expired {
        "expired".to_string()
    } else if verification.valid {
        "passed".to_string()
    } else {
        "failed".to_string()
    };
    response.failed_requirements = verification.errors.clone();
    response.verification_result = Some(serde_json::json!({
        "valid": verification.valid,
        "signature_valid": verification.signature_valid,
        "expired": verification.expired,
        "checked_at": verification.checked_at,
        "warnings": verification.warnings,
        "errors": verification.errors,
    }));
    (response, verification)
}

#[derive(Debug, PartialEq, Eq)]
enum LocalReadinessStatus {
    Uninitialized,
    NotVerified,
    ReadyLocally,
    Failed,
}

struct LocalReadiness {
    status: LocalReadinessStatus,
    messages: Vec<String>,
}

fn classify_local_readiness(_agent_version: &str) -> LocalReadiness {
    let identity_missing = load_identity().is_err();
    let signed_missing = load_latest_signed_report().is_err();
    let system = test_fixtures::system_report();
    let score = test_fixtures::score_report();
    let verification = verify_provider_from_reports(
        load_identity().map(|_| ()),
        &system,
        &score,
        load_latest_signed_report(),
    );
    let mut messages = Vec::new();

    if identity_missing {
        messages.push("identity missing".to_string());
    }
    if signed_missing {
        messages.push("signed report missing".to_string());
    }
    if !verification.challenge_verified {
        messages.push("challenge pending".to_string());
    }

    let status = if identity_missing {
        LocalReadinessStatus::Uninitialized
    } else if verification
        .failed_checks
        .iter()
        .any(|check| check == "report_signature_invalid")
    {
        messages.push("signed report failed".to_string());
        LocalReadinessStatus::Failed
    } else if !signed_missing && verification.signature_verified {
        messages.push("ready locally".to_string());
        LocalReadinessStatus::ReadyLocally
    } else {
        LocalReadinessStatus::NotVerified
    };

    LocalReadiness { status, messages }
}

fn assert_no_secret_fields(value: &Value, token: Option<&str>) {
    let json = serde_json::to_string(value).unwrap();
    for field in [
        "secret_key_base64",
        "private_key_path",
        "api_token_hash",
        "api_token",
        "credentials",
    ] {
        assert!(!json.contains(field), "unexpected secret field {field}");
    }
    if let Some(token) = token {
        assert!(!json.contains(token), "unexpected API token value");
    }
}

fn assert_no_secret_values(value: &Value, token: Option<&str>) {
    let json = serde_json::to_string(value).unwrap();
    let config = load_identity().unwrap();
    let private_key = load_private_key(&config).unwrap();
    assert!(
        !json.contains(&private_key.secret_key_base64),
        "unexpected private key material"
    );
    assert!(
        !json.contains(&config.private_key_path),
        "unexpected private key path"
    );
    if let Some(hash) = config.api_token_hash {
        assert!(!json.contains(&hash), "unexpected API token hash");
    }
    if let Some(token) = token {
        assert!(!json.contains(token), "unexpected API token value");
    }
}

fn assert_contract_snapshot(name: &str, mut value: Value, env: &TestEnv) {
    sanitize_contract_value(&mut value);
    assert_no_secret_values(&value, None);
    let actual = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
    assert!(
        !actual.contains(&env.state_dir.display().to_string()),
        "snapshot contains temporary state path"
    );
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("contracts")
        .join(name);

    if std::env::var_os("BURD_UPDATE_CONTRACT_SNAPSHOTS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &actual).unwrap();
    }

    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("contract snapshot missing at {}: {error}", path.display()));
    assert_eq!(
        actual,
        expected,
        "sanitized contract snapshot changed: {}",
        path.display()
    );
}

fn sanitize_contract_value(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                sanitize_contract_value(item);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if item.is_string() && is_secret_field(key) {
                    *item = Value::String("<redacted>".to_string());
                } else if item.is_string() && is_path_field(key) {
                    *item = Value::String("<path>".to_string());
                } else if item.is_string() && is_timestamp_field(key) {
                    *item = Value::String("<timestamp>".to_string());
                } else if item.is_string() && is_cryptographic_field(key) {
                    *item = Value::String("<cryptographic-value>".to_string());
                } else {
                    sanitize_contract_value(item);
                }
            }
        }
        _ => {}
    }
}

fn is_secret_field(key: &str) -> bool {
    matches!(
        key,
        "secret_key_base64" | "private_key_path" | "api_token_hash" | "api_token" | "credentials"
    )
}

fn is_path_field(key: &str) -> bool {
    matches!(key, "path" | "output" | "config_path")
}

fn is_timestamp_field(key: &str) -> bool {
    matches!(
        key,
        "timestamp"
            | "created_at"
            | "last_check_date"
            | "last_seen_at"
            | "last_online_at"
            | "last_failed_check_at"
            | "signed_at"
            | "checked_at"
            | "issued_at"
            | "expires_at"
            | "completed_at"
    )
}

fn is_cryptographic_field(key: &str) -> bool {
    matches!(key, "public_key" | "signature") || key.ends_with("report_hash")
}

struct TestEnv {
    previous_home: Option<OsString>,
    previous_config: Option<OsString>,
    state_dir: PathBuf,
    config_path: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let state_dir = unique_temp_dir(label);
        fs::create_dir_all(&state_dir).unwrap();
        let config_path = state_dir.join("agent.json");
        let previous_home = std::env::var_os("BURD_AGENT_HOME");
        let previous_config = std::env::var_os("BURD_AGENT_CONFIG");

        // SAFETY: every test in this module holds ENV_LOCK while mutating
        // process environment variables, and the tests spawn no background work.
        unsafe {
            std::env::set_var("BURD_AGENT_HOME", &state_dir);
            std::env::set_var("BURD_AGENT_CONFIG", &config_path);
        }

        Self {
            previous_home,
            previous_config,
            state_dir,
            config_path,
        }
    }

    fn assert_active(&self) {
        assert!(default_state_dir().starts_with(&self.state_dir));
        assert_eq!(default_config_path(), self.config_path);
    }

    fn path(&self, child: &str) -> PathBuf {
        self.state_dir.join(child)
    }

    fn install_identity(&self) {
        let private_key_path = self.state_dir.join("agent.key");
        let config = serde_json::json!({
            "provider_id": test_fixtures::FIXTURE_PROVIDER_ID,
            "machine_id": test_fixtures::FIXTURE_MACHINE_ID,
            "api_url": "https://api.burd.cloud",
            "preferred_provider": "ollama",
            "benchmark_profile": "auto",
            "telemetry_enabled": false,
            "created_at": test_fixtures::FIXTURE_TIMESTAMP,
            "public_key": test_fixtures::FIXTURE_PUBLIC_KEY,
            "key_algorithm": "ed25519",
            "private_key_path": private_key_path,
            "email": null,
            "website": null,
            "country": "BR",
            "city": "Sao Paulo",
            "region": "br-southeast",
            "api_token_hash": null,
            "api_auth_enabled": false,
            "api_bind_host": "127.0.0.1",
            "api_port": 8787,
            "default_network_endpoint": "https://www.cloudflare.com/cdn-cgi/trace"
        });
        let private_key = serde_json::json!({
            "key_algorithm": "ed25519",
            "secret_key_base64": test_fixtures::FIXTURE_SECRET_KEY,
            "created_at": test_fixtures::FIXTURE_TIMESTAMP,
        });
        fs::write(
            &self.config_path,
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
        fs::write(
            private_key_path,
            serde_json::to_string_pretty(&private_key).unwrap(),
        )
        .unwrap();
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        // SAFETY: tests that create TestEnv serialize env var mutation through
        // ENV_LOCK, so restore/remove operations cannot race with peer tests.
        unsafe {
            if let Some(value) = &self.previous_home {
                std::env::set_var("BURD_AGENT_HOME", value);
            } else {
                std::env::remove_var("BURD_AGENT_HOME");
            }

            if let Some(value) = &self.previous_config {
                std::env::set_var("BURD_AGENT_CONFIG", value);
            } else {
                std::env::remove_var("BURD_AGENT_CONFIG");
            }
        }

        let _ = fs::remove_dir_all(&self.state_dir);
    }
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let safe_label: String = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "burd-bench-contract-{}-{safe_label}-{nanos}",
        std::process::id()
    ))
}
