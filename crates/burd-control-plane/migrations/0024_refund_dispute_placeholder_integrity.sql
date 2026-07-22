DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_refunds_amount_positive'
          AND conrelid = 'billing_refunds'::regclass
    ) THEN
        ALTER TABLE billing_refunds
            ADD CONSTRAINT billing_refunds_amount_positive
            CHECK (amount_micros > 0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_refunds_currency_format'
          AND conrelid = 'billing_refunds'::regclass
    ) THEN
        ALTER TABLE billing_refunds
            ADD CONSTRAINT billing_refunds_currency_format
            CHECK (currency ~ '^[A-Z]{3}$');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_refunds_status_allowed'
          AND conrelid = 'billing_refunds'::regclass
    ) THEN
        ALTER TABLE billing_refunds
            ADD CONSTRAINT billing_refunds_status_allowed
            CHECK (status IN ('requested', 'approved', 'rejected', 'settled', 'cancelled'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_refunds_reason_not_blank'
          AND conrelid = 'billing_refunds'::regclass
    ) THEN
        ALTER TABLE billing_refunds
            ADD CONSTRAINT billing_refunds_reason_not_blank
            CHECK (btrim(reason) <> '');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_disputes_hold_amount_positive'
          AND conrelid = 'billing_disputes'::regclass
    ) THEN
        ALTER TABLE billing_disputes
            ADD CONSTRAINT billing_disputes_hold_amount_positive
            CHECK (hold_amount_micros > 0);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_disputes_currency_format'
          AND conrelid = 'billing_disputes'::regclass
    ) THEN
        ALTER TABLE billing_disputes
            ADD CONSTRAINT billing_disputes_currency_format
            CHECK (currency ~ '^[A-Z]{3}$');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_disputes_status_allowed'
          AND conrelid = 'billing_disputes'::regclass
    ) THEN
        ALTER TABLE billing_disputes
            ADD CONSTRAINT billing_disputes_status_allowed
            CHECK (status IN ('opened', 'under_review', 'accepted', 'rejected', 'cancelled', 'closed'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_disputes_reason_not_blank'
          AND conrelid = 'billing_disputes'::regclass
    ) THEN
        ALTER TABLE billing_disputes
            ADD CONSTRAINT billing_disputes_reason_not_blank
            CHECK (btrim(reason) <> '');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_reconciliation_events_status_allowed'
          AND conrelid = 'billing_reconciliation_events'::regclass
    ) THEN
        ALTER TABLE billing_reconciliation_events
            ADD CONSTRAINT billing_reconciliation_events_status_allowed
            CHECK (status IN ('recorded', 'matched', 'ignored', 'conflict', 'rejected'));
    END IF;
END;
$$;

CREATE INDEX IF NOT EXISTS idx_billing_refunds_project_status
    ON billing_refunds(project_id, status, created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_billing_disputes_open_invoice
    ON billing_disputes(invoice_id)
    WHERE status IN ('opened', 'under_review');

CREATE INDEX IF NOT EXISTS idx_billing_disputes_project_status
    ON billing_disputes(project_id, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_billing_reconciliation_status
    ON billing_reconciliation_events(provider, status, created_at DESC);