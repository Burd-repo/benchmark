CREATE TABLE IF NOT EXISTS organizations (
    organization_id TEXT PRIMARY KEY,
    schema_version TEXT NOT NULL,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS organization_users (
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    user_id TEXT NOT NULL REFERENCES users(user_id),
    role TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (organization_id, user_id)
);

CREATE TABLE IF NOT EXISTS projects (
    project_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    schema_version TEXT NOT NULL,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS project_quotas (
    project_id TEXT PRIMARY KEY REFERENCES projects(project_id),
    max_active_reservations INTEGER NOT NULL,
    max_reserved_gpu_seconds BIGINT NOT NULL,
    max_reservation_ttl_seconds INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS customer_api_keys (
    api_key_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    project_id TEXT REFERENCES projects(project_id),
    schema_version TEXT NOT NULL,
    key_prefix TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,
    scopes_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    expires_at TEXT,
    revoked_at TEXT
);

CREATE TABLE IF NOT EXISTS customer_credit_ledger_entries (
    credit_entry_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    schema_version TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    amount_credits BIGINT NOT NULL,
    reservation_id TEXT,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS marketplace_reservations (
    reservation_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    listing_id TEXT NOT NULL REFERENCES marketplace_listings(listing_id),
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    session_id TEXT REFERENCES provider_sessions(session_id),
    schema_version TEXT NOT NULL,
    workload_type TEXT NOT NULL,
    gpu_uuid TEXT,
    status TEXT NOT NULL,
    idempotency_key TEXT,
    request_hash TEXT,
    starts_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    cancelled_at TEXT,
    reserved_gpu_seconds BIGINT NOT NULL,
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(project_id, idempotency_key)
);

CREATE TABLE IF NOT EXISTS customer_audit_events (
    customer_audit_event_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    project_id TEXT REFERENCES projects(project_id),
    schema_version TEXT NOT NULL,
    actor_type TEXT NOT NULL,
    actor_id TEXT,
    event_type TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    occurred_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_marketplace_reservations_active_listing
    ON marketplace_reservations(listing_id)
    WHERE status = 'reserved';
CREATE INDEX IF NOT EXISTS idx_marketplace_reservations_project_status
    ON marketplace_reservations(project_id, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_marketplace_reservations_provider
    ON marketplace_reservations(provider_id, device_id, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_customer_api_keys_project_status
    ON customer_api_keys(project_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_customer_credit_ledger_project_time
    ON customer_credit_ledger_entries(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_customer_audit_events_org_time
    ON customer_audit_events(organization_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_projects_organization_status
    ON projects(organization_id, status, updated_at DESC);

CREATE OR REPLACE FUNCTION prevent_customer_credit_ledger_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'customer_credit_ledger_entries is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS customer_credit_ledger_no_update ON customer_credit_ledger_entries;
CREATE TRIGGER customer_credit_ledger_no_update
    BEFORE UPDATE OR DELETE ON customer_credit_ledger_entries
    FOR EACH ROW EXECUTE FUNCTION prevent_customer_credit_ledger_mutation();