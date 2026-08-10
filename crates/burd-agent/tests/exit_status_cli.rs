use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn invalid_remote_session_invocation_uses_exit_code_two() {
    let (output, root) = run_agent(
        "invalid-invocation",
        &[
            "remote-session",
            "connect",
            "--telemetry-batch-samples",
            "0",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let event = exit_event(&output);
    assert_eq!(event["schema_version"], "burd.agent.exit.v1");
    assert_eq!(event["category"], "invalid_invocation");
    assert_eq!(event["exit_code"], 2);
    assert_eq!(event["failure_kind"], "telemetry_config");
    assert_redacted(&output);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn disabled_provider_job_worker_rejects_canary_configuration() {
    let (output, root) = run_agent(
        "disabled-provider-job-worker",
        &[
            "remote-session",
            "connect",
            "--provider-job-image",
            "llm_inference=ghcr.io/burd/canary@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let event = exit_event(&output);
    assert_eq!(event["category"], "invalid_invocation");
    assert_eq!(event["exit_code"], 2);
    assert_eq!(event["failure_kind"], "provider_job_canary_config");
    assert_redacted(&output);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_local_identity_uses_local_state_exit_and_lifecycle() {
    let (output, root) = run_agent("missing-identity", &["remote-session", "connect"]);
    assert_eq!(output.status.code(), Some(10));
    let event = exit_event(&output);
    assert_eq!(event["category"], "local_state");
    assert_eq!(event["exit_code"], 10);
    assert_eq!(event["failure_kind"], "local_state");
    assert_redacted(&output);

    let lifecycle: Value = serde_json::from_slice(
        &std::fs::read(root.join("agent-lifecycle.json"))
            .expect("missing lifecycle state after terminal startup failure"),
    )
    .unwrap();
    assert_eq!(lifecycle["phase"], "terminal_failure");
    assert_eq!(lifecycle["ready"], false);
    assert_eq!(lifecycle["failure_kind"], "local_state");
    let _ = std::fs::remove_dir_all(root);
}

fn run_agent(label: &str, args: &[&str]) -> (Output, PathBuf) {
    let root = test_root(label);
    let config = root.join("agent.json");
    let output = Command::new(env!("CARGO_BIN_EXE_burd-agent"))
        .args(args)
        .env("BURD_AGENT_CONFIG", &config)
        .env("BURD_ENROLLMENT_TOKEN", "must-not-leak-enrollment")
        .env("BURD_API_TOKEN", "must-not-leak-api-token")
        .output()
        .expect("failed to run burd-agent");
    (output, root)
}

fn exit_event(output: &Output) -> Value {
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    let line = stderr
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("missing structured exit event in stderr: {stderr}"));
    serde_json::from_str(line).expect("invalid structured exit event")
}

fn assert_redacted(output: &Output) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("must-not-leak-enrollment"));
    assert!(!stderr.contains("must-not-leak-api-token"));
    assert!(!stderr.contains("Authorization"));
    assert!(!stderr.contains("Bearer"));
}

fn test_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "burd-agent-exit-{label}-{}-{nonce}",
        std::process::id()
    ))
}
