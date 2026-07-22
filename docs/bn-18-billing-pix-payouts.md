# BN-18 - Billing, Pix And Payouts

BN-18 adds the first backend-owned financial layer on top of BN-15 usage receipts, BN-16 listings, and BN-17 customer reservations. It introduces billing primitives without coupling Burd accounting to a specific Pix provider.

Implemented scope:

- `marketplace_listing_prices`, an admin-managed active price book for marketplace listings;
- `pix_payment_intents`, customer-created payment intents that do not affect balances until confirmed by an admin/webhook adapter;
- `financial_ledger_lines`, an append-only double-entry ledger table;
- `billing_invoices`, generated from a reservation plus a BN-15 usage ledger entry;
- customer project balances and financial ledger listing;
- provider payable balances and financial ledger listing;
- `provider_payout_accounts` with Pix method, hashed Pix key material, KYC status, tax status, minimum payout, and hold days;
- `provider_payouts`, created from provider payable balance with payout clearing ledger lines;
- schema placeholders for refunds, disputes, chargeback reserve, and reconciliation events.

## Financial Ledger

`financial_ledger_lines` is append-only. Database triggers reject update/delete, so corrections must be represented by future compensating entries.

Every transaction written by the control plane balances to zero before insert. Examples:

- Pix confirmation:
  - `customer_balance +amount`
  - `payment_processor_clearing -amount`
- invoice settlement:
  - `customer_balance -subtotal`
  - `provider_payable +provider_net`
  - `platform_revenue +platform_fee`
  - `chargeback_reserve +reserve`
- provider payout:
  - `provider_payable -amount`
  - `provider_payout_clearing +amount`

## Pix Adapter Boundary

BN-18 does not call a real Pix gateway. The API stores intents and allows an admin/webhook adapter to confirm an external Pix payment reference. The ledger only changes on first confirmation. Reconfirming an already confirmed intent is idempotent only when the provider, external reference, and supplied `paid_at` match the stored confirmation; conflicting confirmation data returns `409 Conflict` and does not append ledger lines.

This keeps the ledger independent from payment providers. Replacing a Pix provider should change adapter code, not financial accounting.

## API Error Contract

BN-18 HTTP errors use the shared control-plane error envelope: `{ "error": { "code": "conflict", "message": "...", "request_id": "req_example", "retry_after_seconds": null, "details": {} } }`. OpenAPI documents the `Idempotency-Key` header only on endpoints that actually require it, and distinguishes `idempotency_conflict` from normal billing or payout `conflict` responses.

## Authorization Boundary

BN-18 separates operator/admin actions from customer project billing actions.

- Admin bearer authorization is required for listing price writes, Pix confirmation, reservation settlement, invoice reads, provider balance/ledger reads, payout account upsert, and payout creation.
- Customer API keys are accepted only on project-scoped customer billing endpoints. Pix payment-intent creation requires `billing:write`; project balance and ledger reads require `billing:read`.
- Customer API keys cannot administer provider payout, settlement, invoice, or price-book endpoints. Provider/device session credentials cannot call customer or admin billing endpoints.

## Billing Settlement

`POST /v1/billing/reservations/{reservation_id}/settle` requires:

- a customer reservation;
- a matching BN-15 usage ledger entry;
- provider/device/GPU binding consistency;
- an active marketplace listing price;
- sufficient confirmed project customer balance in the billing currency.

The backend calculates billable amount from usage `billable_gpu_seconds` and the listing price, then requires enough confirmed project balance before issuing the invoice. The provider cannot submit billing amount, platform fee, payout amount, or customer balance. A BN-15 usage ledger entry can settle into at most one billing invoice: replaying the same reservation/usage pair returns the existing invoice as `duplicate=true`, while attempting to settle that usage entry against another reservation returns `409 Conflict`.

## Payouts

Provider payout creation requires:

- an active Pix payout account;
- KYC status `verified`;
- tax status `verified`;
- payable balance greater than or equal to requested amount;
- amount greater than or equal to minimum payout.

Payout hold days move the payout into `held` status while the ledger immediately reserves provider payable into payout clearing. Hardening migration `0023_provider_payout_reconciliation_integrity` enforces payout account status, payout status, positive amounts, currency format, held payout hold timestamps, paid payout reference requirements, and unique payout external references at the database boundary.

## Reconciliation Placeholder

`billing_reconciliation_events` remains a schema placeholder for future payment and payout reconciliation ingestion. Migration `0023_provider_payout_reconciliation_integrity` only adds basic integrity constraints and lookup indexes; it does not add webhook ingestion, automatic matching, banking payout execution, or settlement release behavior.

## Not Implemented Yet

BN-18 still does not implement:

- live Pix provider API calls;
- automatic webhook signature verification;
- automatic invoice collection policy;
- customer self-serve refunds or disputes;
- payout execution through a banking provider;
- tax documents, KYC vendor integration, or legal compliance workflow;
- full checkout UI.
