CREATE TABLE IF NOT EXISTS marketplace_listing_prices (
    price_id TEXT PRIMARY KEY,
    listing_id TEXT NOT NULL REFERENCES marketplace_listings(listing_id),
    schema_version TEXT NOT NULL,
    currency TEXT NOT NULL,
    price_per_hour_micros BIGINT NOT NULL,
    pricing_model TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_marketplace_listing_prices_active
    ON marketplace_listing_prices(listing_id)
    WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_marketplace_listing_prices_listing_time
    ON marketplace_listing_prices(listing_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS pix_payment_intents (
    payment_intent_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    schema_version TEXT NOT NULL,
    status TEXT NOT NULL,
    amount_micros BIGINT NOT NULL,
    currency TEXT NOT NULL,
    provider TEXT NOT NULL,
    external_reference TEXT,
    adapter_status TEXT NOT NULL,
    idempotency_key TEXT,
    request_hash TEXT,
    confirmed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(project_id, idempotency_key)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_pix_payment_intents_provider_reference
    ON pix_payment_intents(provider, external_reference)
    WHERE external_reference IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_pix_payment_intents_project_status
    ON pix_payment_intents(project_id, status, updated_at DESC);

CREATE TABLE IF NOT EXISTS billing_invoices (
    invoice_id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    reservation_id TEXT REFERENCES marketplace_reservations(reservation_id),
    usage_entry_id TEXT REFERENCES usage_ledger_entries(entry_id),
    schema_version TEXT NOT NULL,
    status TEXT NOT NULL,
    currency TEXT NOT NULL,
    subtotal_micros BIGINT NOT NULL,
    platform_fee_micros BIGINT NOT NULL,
    provider_net_micros BIGINT NOT NULL,
    chargeback_reserve_micros BIGINT NOT NULL,
    total_micros BIGINT NOT NULL,
    source_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(reservation_id, usage_entry_id)
);

CREATE INDEX IF NOT EXISTS idx_billing_invoices_project_status
    ON billing_invoices(project_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_billing_invoices_usage
    ON billing_invoices(usage_entry_id);

CREATE TABLE IF NOT EXISTS financial_ledger_lines (
    ledger_line_id TEXT PRIMARY KEY,
    transaction_id TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    line_number INTEGER NOT NULL,
    account_type TEXT NOT NULL,
    account_owner_type TEXT NOT NULL,
    account_owner_id TEXT,
    currency TEXT NOT NULL,
    amount_micros BIGINT NOT NULL,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(transaction_id, line_number)
);

CREATE INDEX IF NOT EXISTS idx_financial_ledger_owner
    ON financial_ledger_lines(account_owner_type, account_owner_id, currency, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_financial_ledger_source
    ON financial_ledger_lines(source_type, source_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_financial_ledger_transaction
    ON financial_ledger_lines(transaction_id, line_number);

CREATE TABLE IF NOT EXISTS provider_payout_accounts (
    payout_account_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    schema_version TEXT NOT NULL,
    payout_method TEXT NOT NULL,
    currency TEXT NOT NULL,
    pix_key_hash TEXT NOT NULL,
    pix_key_last4 TEXT NOT NULL,
    kyc_status TEXT NOT NULL,
    tax_status TEXT NOT NULL,
    minimum_payout_micros BIGINT NOT NULL,
    payout_hold_days INTEGER NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(provider_id, payout_method, currency)
);

CREATE INDEX IF NOT EXISTS idx_provider_payout_accounts_provider
    ON provider_payout_accounts(provider_id, status, updated_at DESC);

CREATE TABLE IF NOT EXISTS provider_payouts (
    payout_id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(provider_id),
    payout_account_id TEXT NOT NULL REFERENCES provider_payout_accounts(payout_account_id),
    schema_version TEXT NOT NULL,
    status TEXT NOT NULL,
    amount_micros BIGINT NOT NULL,
    currency TEXT NOT NULL,
    hold_until TEXT,
    external_reference TEXT,
    paid_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_provider_payouts_provider_status
    ON provider_payouts(provider_id, status, created_at DESC);

CREATE TABLE IF NOT EXISTS billing_refunds (
    refund_id TEXT PRIMARY KEY,
    invoice_id TEXT NOT NULL REFERENCES billing_invoices(invoice_id),
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    schema_version TEXT NOT NULL,
    status TEXT NOT NULL,
    amount_micros BIGINT NOT NULL,
    currency TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_billing_refunds_invoice
    ON billing_refunds(invoice_id, created_at DESC);

CREATE TABLE IF NOT EXISTS billing_disputes (
    dispute_id TEXT PRIMARY KEY,
    invoice_id TEXT NOT NULL REFERENCES billing_invoices(invoice_id),
    organization_id TEXT NOT NULL REFERENCES organizations(organization_id),
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    schema_version TEXT NOT NULL,
    status TEXT NOT NULL,
    reason TEXT NOT NULL,
    hold_amount_micros BIGINT NOT NULL,
    currency TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_billing_disputes_invoice
    ON billing_disputes(invoice_id, status, created_at DESC);

CREATE TABLE IF NOT EXISTS billing_reconciliation_events (
    reconciliation_event_id TEXT PRIMARY KEY,
    schema_version TEXT NOT NULL,
    provider TEXT NOT NULL,
    external_reference TEXT NOT NULL,
    amount_micros BIGINT NOT NULL,
    currency TEXT NOT NULL,
    event_type TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(provider, external_reference, event_type)
);

CREATE INDEX IF NOT EXISTS idx_billing_reconciliation_provider_time
    ON billing_reconciliation_events(provider, created_at DESC);

CREATE OR REPLACE FUNCTION prevent_financial_ledger_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'financial_ledger_lines is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS financial_ledger_no_update ON financial_ledger_lines;
CREATE TRIGGER financial_ledger_no_update
    BEFORE UPDATE OR DELETE ON financial_ledger_lines
    FOR EACH ROW EXECUTE FUNCTION prevent_financial_ledger_mutation();
