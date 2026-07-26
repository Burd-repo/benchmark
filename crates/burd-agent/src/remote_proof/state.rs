use burd_protocol::{default_state_dir, random_token};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const PROOF_ATTEMPT_STATE_SCHEMA_VERSION: &str = "1";
const PROOF_ATTEMPT_HISTORY_LIMIT: usize = 64;
const PROOF_ATTEMPT_STATE_MAX_BYTES: u64 = 256 * 1024;
const PROOF_ATTEMPT_ID_MAX_BYTES: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProofAttemptOutcome {
    RejectedLocally,
    AttemptFailed,
    Submitted,
}

impl ProofAttemptOutcome {
    fn suppresses_retry(self) -> bool {
        matches!(self, Self::RejectedLocally | Self::AttemptFailed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProofAttemptRecord {
    pub(crate) challenge_id: String,
    pub(crate) session_id: String,
    pub(crate) outcome: ProofAttemptOutcome,
    pub(crate) recorded_at: String,
    pub(crate) suppress_until: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProofAttemptState {
    schema_version: String,
    attempts: Vec<ProofAttemptRecord>,
}

impl Default for ProofAttemptState {
    fn default() -> Self {
        Self {
            schema_version: PROOF_ATTEMPT_STATE_SCHEMA_VERSION.to_string(),
            attempts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProofAttemptStore {
    path: PathBuf,
    state: ProofAttemptState,
}

impl ProofAttemptStore {
    pub(crate) fn load_default() -> Result<Self, String> {
        Self::load_from(proof_attempt_state_path())
    }

    fn load_from(path: PathBuf) -> Result<Self, String> {
        let state = match fs::metadata(&path) {
            Ok(metadata) => {
                if metadata.len() > PROOF_ATTEMPT_STATE_MAX_BYTES {
                    return Err(format!(
                        "proof attempt state at {} exceeds {} bytes",
                        path.display(),
                        PROOF_ATTEMPT_STATE_MAX_BYTES
                    ));
                }
                let bytes = fs::read(&path).map_err(|error| {
                    format!(
                        "failed to read proof attempt state at {}: {error}",
                        path.display()
                    )
                })?;
                serde_json::from_slice::<ProofAttemptState>(&bytes).map_err(|error| {
                    format!(
                        "failed to parse proof attempt state at {}: {error}",
                        path.display()
                    )
                })?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ProofAttemptState::default()
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect proof attempt state at {}: {error}",
                    path.display()
                ));
            }
        };
        validate_state(&state)?;
        Ok(Self { path, state })
    }

    pub(crate) fn active_suppression(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Option<&ProofAttemptRecord> {
        self.state.attempts.iter().rev().find(|attempt| {
            attempt.session_id == session_id
                && attempt.outcome.suppresses_retry()
                && parse_timestamp(&attempt.suppress_until).is_ok_and(|until| until > now)
        })
    }

    pub(crate) fn record(
        &mut self,
        challenge_id: String,
        session_id: String,
        outcome: ProofAttemptOutcome,
        recorded_at: DateTime<Utc>,
        suppress_until: DateTime<Utc>,
    ) -> Result<(), String> {
        validate_identifier("challenge_id", &challenge_id)?;
        validate_identifier("session_id", &session_id)?;
        if suppress_until < recorded_at {
            return Err("proof attempt suppression cannot end before it was recorded".to_string());
        }

        self.state.attempts.retain(|attempt| {
            attempt.challenge_id != challenge_id || attempt.session_id != session_id
        });
        self.state.attempts.push(ProofAttemptRecord {
            challenge_id,
            session_id,
            outcome,
            recorded_at: recorded_at.to_rfc3339(),
            suppress_until: suppress_until.to_rfc3339(),
        });
        if self.state.attempts.len() > PROOF_ATTEMPT_HISTORY_LIMIT {
            let excess = self.state.attempts.len() - PROOF_ATTEMPT_HISTORY_LIMIT;
            self.state.attempts.drain(0..excess);
        }
        write_state_atomic(&self.path, &self.state)
    }
}

fn proof_attempt_state_path() -> PathBuf {
    default_state_dir().join("remote-proof-attempts.json")
}

fn validate_state(state: &ProofAttemptState) -> Result<(), String> {
    if state.schema_version != PROOF_ATTEMPT_STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported proof attempt state schema {}",
            state.schema_version
        ));
    }
    if state.attempts.len() > PROOF_ATTEMPT_HISTORY_LIMIT {
        return Err(format!(
            "proof attempt state contains more than {PROOF_ATTEMPT_HISTORY_LIMIT} records"
        ));
    }
    for attempt in &state.attempts {
        validate_identifier("challenge_id", &attempt.challenge_id)?;
        validate_identifier("session_id", &attempt.session_id)?;
        let recorded_at = parse_timestamp(&attempt.recorded_at)?;
        let suppress_until = parse_timestamp(&attempt.suppress_until)?;
        if suppress_until < recorded_at {
            return Err(
                "proof attempt state contains a suppression before its record time".to_string(),
            );
        }
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > PROOF_ATTEMPT_ID_MAX_BYTES
        || !value.is_ascii()
        || value.chars().any(char::is_whitespace)
    {
        return Err(format!("proof attempt {name} is invalid"));
    }
    Ok(())
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| format!("invalid proof attempt timestamp: {error}"))
}

fn write_state_atomic(path: &Path, state: &ProofAttemptState) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "proof attempt state path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("failed to serialize proof attempt state: {error}"))?;
    if bytes.len() as u64 > PROOF_ATTEMPT_STATE_MAX_BYTES {
        return Err("serialized proof attempt state exceeds its size limit".to_string());
    }

    let temporary_suffix = random_token("proof_state")
        .map_err(|error| format!("failed to generate proof attempt temporary path: {error}"))?;
    let temporary = path.with_extension(format!("json.{temporary_suffix}.tmp"));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| {
                format!(
                    "failed to open temporary proof attempt state at {}: {error}",
                    temporary.display()
                )
            })?;
        file.write_all(&bytes).map_err(|error| {
            format!(
                "failed to write temporary proof attempt state at {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync temporary proof attempt state at {}: {error}",
                temporary.display()
            )
        })?;
        fs::rename(&temporary, path).map_err(|error| {
            format!(
                "failed to replace proof attempt state at {}: {error}",
                path.display()
            )
        })
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn failed_attempt_survives_reload_and_suppresses_same_session() {
        let root = test_root("reload");
        let path = root.join("remote-proof-attempts.json");
        let now = Utc::now();
        let mut store = ProofAttemptStore::load_from(path.clone()).unwrap();
        store
            .record(
                "proof_challenge_1".to_string(),
                "session_1".to_string(),
                ProofAttemptOutcome::AttemptFailed,
                now,
                now + Duration::minutes(10),
            )
            .unwrap();

        let reloaded = ProofAttemptStore::load_from(path).unwrap();
        assert!(
            reloaded
                .active_suppression("session_1", now + Duration::minutes(1))
                .is_some()
        );
        assert!(
            reloaded
                .active_suppression("session_2", now + Duration::minutes(1))
                .is_none()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn submitted_and_expired_attempts_do_not_suppress_retry() {
        let root = test_root("outcomes");
        let path = root.join("remote-proof-attempts.json");
        let now = Utc::now();
        let mut store = ProofAttemptStore::load_from(path).unwrap();
        store
            .record(
                "proof_submitted".to_string(),
                "session_1".to_string(),
                ProofAttemptOutcome::Submitted,
                now,
                now,
            )
            .unwrap();
        store
            .record(
                "proof_expired".to_string(),
                "session_1".to_string(),
                ProofAttemptOutcome::RejectedLocally,
                now - Duration::minutes(2),
                now - Duration::minutes(1),
            )
            .unwrap();

        assert!(store.active_suppression("session_1", now).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn history_is_deduplicated_bounded_and_contains_no_sensitive_payloads() {
        let root = test_root("bounded");
        let path = root.join("remote-proof-attempts.json");
        let now = Utc::now();
        let mut store = ProofAttemptStore::load_from(path.clone()).unwrap();
        for index in 0..(PROOF_ATTEMPT_HISTORY_LIMIT + 4) {
            store
                .record(
                    format!("proof_{index}"),
                    "session_1".to_string(),
                    ProofAttemptOutcome::AttemptFailed,
                    now,
                    now + Duration::minutes(10),
                )
                .unwrap();
        }
        store
            .record(
                "proof_67".to_string(),
                "session_1".to_string(),
                ProofAttemptOutcome::Submitted,
                now,
                now,
            )
            .unwrap();

        let reloaded = ProofAttemptStore::load_from(path.clone()).unwrap();
        assert_eq!(reloaded.state.attempts.len(), PROOF_ATTEMPT_HISTORY_LIMIT);
        assert_eq!(
            reloaded
                .state
                .attempts
                .iter()
                .filter(|attempt| attempt.challenge_id == "proof_67")
                .count(),
            1
        );
        let raw = fs::read_to_string(path).unwrap();
        for forbidden in [
            "nonce",
            "resume_token",
            "credential",
            "signature",
            "private_key",
            "error",
        ] {
            assert!(!raw.contains(forbidden));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_or_oversized_state_fails_closed() {
        let root = test_root("invalid");
        let path = root.join("remote-proof-attempts.json");
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, br#"{"schema_version":"2","attempts":[]}"#).unwrap();
        assert!(ProofAttemptStore::load_from(path.clone()).is_err());

        fs::write(
            &path,
            vec![b' '; PROOF_ATTEMPT_STATE_MAX_BYTES as usize + 1],
        )
        .unwrap();
        assert!(ProofAttemptStore::load_from(path).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "burd-proof-attempt-state-{label}-{}-{nonce}",
            std::process::id()
        ))
    }
}
