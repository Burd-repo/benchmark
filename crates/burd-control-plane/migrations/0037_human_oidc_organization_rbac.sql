CREATE TABLE IF NOT EXISTS human_oidc_identities (
    oidc_identity_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(user_id),
    provider TEXT NOT NULL,
    provider_subject TEXT NOT NULL,
    email TEXT,
    email_verified BOOLEAN NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_login_at TEXT,
    CONSTRAINT human_oidc_identities_provider_known CHECK (provider IN ('google')),
    CONSTRAINT human_oidc_identities_provider_subject_present CHECK (length(provider_subject) BETWEEN 1 AND 255),
    UNIQUE (provider, provider_subject)
);

CREATE TABLE IF NOT EXISTS human_sessions (
    session_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(user_id),
    token_hash TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    last_seen_at TEXT,
    revoked_at TEXT,
    CONSTRAINT human_sessions_status_known CHECK (status IN ('active', 'revoked')),
    CONSTRAINT human_sessions_expiry_after_creation CHECK (expires_at > created_at),
    CONSTRAINT human_sessions_revocation_consistent CHECK (
        (status = 'active' AND revoked_at IS NULL)
        OR (status = 'revoked' AND revoked_at IS NOT NULL)
    )
);

ALTER TABLE organization_users
    ADD CONSTRAINT organization_users_role_known
    CHECK (role IN ('owner', 'admin', 'billing_admin', 'developer', 'viewer'));

ALTER TABLE organization_users
    ADD CONSTRAINT organization_users_status_known
    CHECK (status IN ('active', 'inactive'));

CREATE INDEX IF NOT EXISTS idx_human_oidc_identities_user
    ON human_oidc_identities(user_id, provider);
CREATE INDEX IF NOT EXISTS idx_human_sessions_user_status
    ON human_sessions(user_id, status, expires_at);
CREATE INDEX IF NOT EXISTS idx_organization_users_user_status
    ON organization_users(user_id, status, organization_id);
