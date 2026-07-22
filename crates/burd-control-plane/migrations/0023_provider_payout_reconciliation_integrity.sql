DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'financial_ledger_lines_amount_nonzero'
          AND conrelid = 'financial_ledger_lines'::regclass
    ) THEN
        ALTER TABLE financial_ledger_lines
            ADD CONSTRAINT financial_ledger_lines_amount_nonzero
            CHECK (amount_micros <> 0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'financial_ledger_lines_line_number_positive'
          AND conrelid = 'financial_ledger_lines'::regclass
    ) THEN
        ALTER TABLE financial_ledger_lines
            ADD CONSTRAINT financial_ledger_lines_line_number_positive
            CHECK (line_number > 0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'financial_ledger_lines_currency_format'
          AND conrelid = 'financial_ledger_lines'::regclass
    ) THEN
        ALTER TABLE financial_ledger_lines
            ADD CONSTRAINT financial_ledger_lines_currency_format
            CHECK (currency ~ '^[A-Z]{3}$');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payout_accounts_method_pix'
          AND conrelid = 'provider_payout_accounts'::regclass
    ) THEN
        ALTER TABLE provider_payout_accounts
            ADD CONSTRAINT provider_payout_accounts_method_pix
            CHECK (payout_method = 'pix');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payout_accounts_currency_format'
          AND conrelid = 'provider_payout_accounts'::regclass
    ) THEN
        ALTER TABLE provider_payout_accounts
            ADD CONSTRAINT provider_payout_accounts_currency_format
            CHECK (currency ~ '^[A-Z]{3}$');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payout_accounts_kyc_status'
          AND conrelid = 'provider_payout_accounts'::regclass
    ) THEN
        ALTER TABLE provider_payout_accounts
            ADD CONSTRAINT provider_payout_accounts_kyc_status
            CHECK (kyc_status IN ('pending', 'verified', 'rejected'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payout_accounts_tax_status'
          AND conrelid = 'provider_payout_accounts'::regclass
    ) THEN
        ALTER TABLE provider_payout_accounts
            ADD CONSTRAINT provider_payout_accounts_tax_status
            CHECK (tax_status IN ('pending', 'verified', 'blocked'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payout_accounts_minimum_positive'
          AND conrelid = 'provider_payout_accounts'::regclass
    ) THEN
        ALTER TABLE provider_payout_accounts
            ADD CONSTRAINT provider_payout_accounts_minimum_positive
            CHECK (minimum_payout_micros > 0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payout_accounts_hold_days_range'
          AND conrelid = 'provider_payout_accounts'::regclass
    ) THEN
        ALTER TABLE provider_payout_accounts
            ADD CONSTRAINT provider_payout_accounts_hold_days_range
            CHECK (payout_hold_days >= 0 AND payout_hold_days <= 90);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payout_accounts_status_allowed'
          AND conrelid = 'provider_payout_accounts'::regclass
    ) THEN
        ALTER TABLE provider_payout_accounts
            ADD CONSTRAINT provider_payout_accounts_status_allowed
            CHECK (status IN ('active', 'revoked'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payouts_status_allowed'
          AND conrelid = 'provider_payouts'::regclass
    ) THEN
        ALTER TABLE provider_payouts
            ADD CONSTRAINT provider_payouts_status_allowed
            CHECK (status IN ('held', 'approved', 'paid', 'failed', 'cancelled'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payouts_amount_positive'
          AND conrelid = 'provider_payouts'::regclass
    ) THEN
        ALTER TABLE provider_payouts
            ADD CONSTRAINT provider_payouts_amount_positive
            CHECK (amount_micros > 0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payouts_currency_format'
          AND conrelid = 'provider_payouts'::regclass
    ) THEN
        ALTER TABLE provider_payouts
            ADD CONSTRAINT provider_payouts_currency_format
            CHECK (currency ~ '^[A-Z]{3}$');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payouts_held_requires_hold_until'
          AND conrelid = 'provider_payouts'::regclass
    ) THEN
        ALTER TABLE provider_payouts
            ADD CONSTRAINT provider_payouts_held_requires_hold_until
            CHECK (status <> 'held' OR hold_until IS NOT NULL);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payouts_external_reference_not_blank'
          AND conrelid = 'provider_payouts'::regclass
    ) THEN
        ALTER TABLE provider_payouts
            ADD CONSTRAINT provider_payouts_external_reference_not_blank
            CHECK (external_reference IS NULL OR btrim(external_reference) <> '');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payouts_paid_requires_external_reference'
          AND conrelid = 'provider_payouts'::regclass
    ) THEN
        ALTER TABLE provider_payouts
            ADD CONSTRAINT provider_payouts_paid_requires_external_reference
            CHECK (status <> 'paid' OR (external_reference IS NOT NULL AND paid_at IS NOT NULL));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'provider_payouts_paid_at_requires_paid_status'
          AND conrelid = 'provider_payouts'::regclass
    ) THEN
        ALTER TABLE provider_payouts
            ADD CONSTRAINT provider_payouts_paid_at_requires_paid_status
            CHECK (paid_at IS NULL OR status = 'paid');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_reconciliation_events_amount_positive'
          AND conrelid = 'billing_reconciliation_events'::regclass
    ) THEN
        ALTER TABLE billing_reconciliation_events
            ADD CONSTRAINT billing_reconciliation_events_amount_positive
            CHECK (amount_micros > 0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_reconciliation_events_currency_format'
          AND conrelid = 'billing_reconciliation_events'::regclass
    ) THEN
        ALTER TABLE billing_reconciliation_events
            ADD CONSTRAINT billing_reconciliation_events_currency_format
            CHECK (currency ~ '^[A-Z]{3}$');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_reconciliation_events_provider_not_blank'
          AND conrelid = 'billing_reconciliation_events'::regclass
    ) THEN
        ALTER TABLE billing_reconciliation_events
            ADD CONSTRAINT billing_reconciliation_events_provider_not_blank
            CHECK (btrim(provider) <> '');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_reconciliation_events_external_reference_not_blank'
          AND conrelid = 'billing_reconciliation_events'::regclass
    ) THEN
        ALTER TABLE billing_reconciliation_events
            ADD CONSTRAINT billing_reconciliation_events_external_reference_not_blank
            CHECK (btrim(external_reference) <> '');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_reconciliation_events_event_type_not_blank'
          AND conrelid = 'billing_reconciliation_events'::regclass
    ) THEN
        ALTER TABLE billing_reconciliation_events
            ADD CONSTRAINT billing_reconciliation_events_event_type_not_blank
            CHECK (btrim(event_type) <> '');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_reconciliation_events_status_not_blank'
          AND conrelid = 'billing_reconciliation_events'::regclass
    ) THEN
        ALTER TABLE billing_reconciliation_events
            ADD CONSTRAINT billing_reconciliation_events_status_not_blank
            CHECK (btrim(status) <> '');
    END IF;
END;
$$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_provider_payouts_external_reference
    ON provider_payouts(external_reference)
    WHERE external_reference IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_billing_reconciliation_reference
    ON billing_reconciliation_events(provider, external_reference, created_at DESC);