CREATE UNIQUE INDEX IF NOT EXISTS idx_customer_credit_ledger_reservation_entry
    ON customer_credit_ledger_entries(reservation_id, entry_type)
    WHERE reservation_id IS NOT NULL
      AND entry_type IN ('reservation_hold', 'reservation_release');