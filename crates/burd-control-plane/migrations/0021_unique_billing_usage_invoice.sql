CREATE UNIQUE INDEX IF NOT EXISTS idx_billing_invoices_unique_usage_entry
    ON billing_invoices(usage_entry_id)
    WHERE usage_entry_id IS NOT NULL;