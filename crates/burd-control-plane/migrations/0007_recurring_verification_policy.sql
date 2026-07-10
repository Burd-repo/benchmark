ALTER TABLE proof_challenges
    ADD COLUMN IF NOT EXISTS trigger_reason TEXT,
    ADD COLUMN IF NOT EXISTS risk_reasons_json TEXT,
    ADD COLUMN IF NOT EXISTS verification_policy_version TEXT;

CREATE TABLE IF NOT EXISTS provider_verification_states (
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    status TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    reason TEXT,
    risk_score DOUBLE PRECISION NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    retry_budget_remaining INTEGER NOT NULL DEFAULT 0,
    last_challenge_id TEXT REFERENCES proof_challenges(challenge_id),
    last_verified_challenge_id TEXT REFERENCES proof_challenges(challenge_id),
    last_verified_at TEXT,
    last_failed_at TEXT,
    last_failure_reason TEXT,
    next_due_at TEXT,
    quarantined_at TEXT,
    blocked_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (provider_id, device_id)
);

CREATE INDEX IF NOT EXISTS idx_provider_verification_states_status_due
    ON provider_verification_states(status, next_due_at);
CREATE INDEX IF NOT EXISTS idx_provider_verification_states_provider
    ON provider_verification_states(provider_id, status);
CREATE INDEX IF NOT EXISTS idx_proof_challenges_policy_reason
    ON proof_challenges(verification_policy_version, trigger_reason, issued_at);
