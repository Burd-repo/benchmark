CREATE TABLE IF NOT EXISTS pricing_snapshots (
    pricing_snapshot_id TEXT PRIMARY KEY,
    source_price_id TEXT NOT NULL REFERENCES marketplace_listing_prices(price_id),
    listing_id TEXT NOT NULL REFERENCES marketplace_listings(listing_id),
    schema_version TEXT NOT NULL,
    currency TEXT NOT NULL,
    pricing_model TEXT NOT NULL,
    price_per_hour_micros BIGINT NOT NULL,
    platform_fee_bps INTEGER NOT NULL,
    chargeback_reserve_bps INTEGER NOT NULL,
    fee_policy_version TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CONSTRAINT pricing_snapshots_currency_format CHECK (currency ~ '^[A-Z]{3}$'),
    CONSTRAINT pricing_snapshots_price_positive CHECK (price_per_hour_micros > 0),
    CONSTRAINT pricing_snapshots_fee_bps_range CHECK (
        platform_fee_bps >= 0
        AND chargeback_reserve_bps >= 0
        AND platform_fee_bps + chargeback_reserve_bps <= 10000
    ),
    CONSTRAINT pricing_snapshots_fee_policy_not_blank CHECK (btrim(fee_policy_version) <> '')
);

ALTER TABLE marketplace_reservations
    ADD COLUMN IF NOT EXISTS pricing_snapshot_id TEXT
        REFERENCES pricing_snapshots(pricing_snapshot_id);

ALTER TABLE usage_ledger_entries
    ADD COLUMN IF NOT EXISTS reservation_id TEXT
        REFERENCES marketplace_reservations(reservation_id);

ALTER TABLE usage_ledger_entries
    ADD COLUMN IF NOT EXISTS pricing_snapshot_id TEXT
        REFERENCES pricing_snapshots(pricing_snapshot_id);

ALTER TABLE billing_invoices
    ADD COLUMN IF NOT EXISTS pricing_snapshot_id TEXT
        REFERENCES pricing_snapshots(pricing_snapshot_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_marketplace_listing_prices_price_listing
    ON marketplace_listing_prices(price_id, listing_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_marketplace_reservations_pricing_snapshot
    ON marketplace_reservations(pricing_snapshot_id)
    WHERE pricing_snapshot_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_marketplace_reservations_snapshot_binding
    ON marketplace_reservations(reservation_id, pricing_snapshot_id);

CREATE INDEX IF NOT EXISTS idx_usage_ledger_reservation_snapshot
    ON usage_ledger_entries(reservation_id, pricing_snapshot_id, created_at DESC)
    WHERE reservation_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_billing_invoices_pricing_snapshot
    ON billing_invoices(pricing_snapshot_id)
    WHERE pricing_snapshot_id IS NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'pricing_snapshots_source_price_listing_fk'
          AND conrelid = 'pricing_snapshots'::regclass
    ) THEN
        ALTER TABLE pricing_snapshots
            ADD CONSTRAINT pricing_snapshots_source_price_listing_fk
            FOREIGN KEY (source_price_id, listing_id)
            REFERENCES marketplace_listing_prices(price_id, listing_id);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'marketplace_reservations_v2_requires_pricing_snapshot'
          AND conrelid = 'marketplace_reservations'::regclass
    ) THEN
        ALTER TABLE marketplace_reservations
            ADD CONSTRAINT marketplace_reservations_v2_requires_pricing_snapshot
            CHECK (schema_version <> 'burd-marketplace-reservation-v2' OR pricing_snapshot_id IS NOT NULL);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'usage_ledger_reservation_snapshot_pair'
          AND conrelid = 'usage_ledger_entries'::regclass
    ) THEN
        ALTER TABLE usage_ledger_entries
            ADD CONSTRAINT usage_ledger_reservation_snapshot_pair
            CHECK ((reservation_id IS NULL AND pricing_snapshot_id IS NULL) OR (reservation_id IS NOT NULL AND pricing_snapshot_id IS NOT NULL));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'usage_ledger_reservation_snapshot_binding_fk'
          AND conrelid = 'usage_ledger_entries'::regclass
    ) THEN
        ALTER TABLE usage_ledger_entries
            ADD CONSTRAINT usage_ledger_reservation_snapshot_binding_fk
            FOREIGN KEY (reservation_id, pricing_snapshot_id)
            REFERENCES marketplace_reservations(reservation_id, pricing_snapshot_id);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_invoices_reservation_snapshot_pair'
          AND conrelid = 'billing_invoices'::regclass
    ) THEN
        ALTER TABLE billing_invoices
            ADD CONSTRAINT billing_invoices_reservation_snapshot_pair
            CHECK ((reservation_id IS NULL AND pricing_snapshot_id IS NULL) OR (reservation_id IS NOT NULL AND pricing_snapshot_id IS NOT NULL)) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'billing_invoices_reservation_snapshot_binding_fk'
          AND conrelid = 'billing_invoices'::regclass
    ) THEN
        ALTER TABLE billing_invoices
            ADD CONSTRAINT billing_invoices_reservation_snapshot_binding_fk
            FOREIGN KEY (reservation_id, pricing_snapshot_id)
            REFERENCES marketplace_reservations(reservation_id, pricing_snapshot_id);
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION prevent_pricing_snapshot_mutation()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'pricing_snapshots is immutable';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS pricing_snapshots_no_update ON pricing_snapshots;
CREATE TRIGGER pricing_snapshots_no_update
    BEFORE UPDATE OR DELETE ON pricing_snapshots
    FOR EACH ROW EXECUTE FUNCTION prevent_pricing_snapshot_mutation();
