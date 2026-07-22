# Customer Billing Ledger Hardening

## Summary

This pass audited the implemented BN-18 customer billing primitives. It did not add a real Pix gateway, bank payouts, checkout UI, marketplace orchestration, provider-side paid execution, or new product flows.

## Scope Audited

- Pix payment intent confirmation behavior
- billing invoice settlement from BN-15 usage ledger entries
- financial ledger append behavior around duplicate billing paths
- billing invoice schema constraints and migration registration
- BN-18 and current-state documentation claims

## Implemented For Real

- Customer-created Pix payment intents exist as backend records and are credited only after admin/adapter confirmation.
- Financial ledger lines are append-only and balanced per financial transaction.
- Reservation billing settlement creates invoices from an existing reservation, usage ledger entry, active listing price, and confirmed project balance.
- Provider payable balances and payout account/payout primitives exist as backend accounting records.

## Still Planned, Mock, Or Local

- No live Pix provider API is called.
- No webhook signature verification or bank payout execution exists.
- No checkout UI, production KYC/tax workflow, refunds/dispute workflow, or external reconciliation automation exists.
- Billing remains tied to the current single-provider usage settlement primitives rather than a complete production marketplace checkout flow.

## Bugs Found

- A confirmed Pix payment intent could be reconfirmed as a duplicate even when the caller supplied a different provider reference. That hid conflicting payment evidence and made the duplicate response too permissive.
- Billing settlement prevented duplicate invoices for the same `(reservation_id, usage_entry_id)` pair, but did not prevent one `usage_entry_id` from being reused to invoice a second compatible reservation.

## Bugs Fixed

- Confirmed Pix intents now accept duplicate confirmation only when the stored provider reference and supplied `paid_at` match the original confirmation. Conflicting confirmation data returns `409 Conflict` and does not append ledger lines.
- Billing settlement now loads existing invoices by `usage_entry_id`. Same-reservation retries return the existing invoice with `duplicate=true`; cross-reservation attempts return `409 Conflict`.
- Invoice insertion now relies on the database uniqueness boundary and only appends financial ledger lines when the invoice insert wins.

## Overengineering Removed

- No new abstraction was added. The hardening stays in the existing explicit transaction flow and helper functions.
- No event bus, listener layer, or async dispatch path was introduced.

## Events And Listeners

- No event bus or listener plumbing was changed.
- Customer audit events and control-plane audit events remain explicit writes inside the existing transactions.

## Migrations And Database

- Added `0021_unique_billing_usage_invoice.sql`.
- The migration creates a partial unique index on `billing_invoices(usage_entry_id)` where `usage_entry_id IS NOT NULL`.
- Existing migrations were not edited.
- The stricter index makes the database enforce the intended one-usage-entry-to-one-invoice invariant.

## Security Findings

- Conflicting Pix confirmation evidence is now rejected instead of treated as a benign duplicate.
- The change does not log Pix keys, authorization headers, private keys, raw tokens, or payment secrets.
- This PR does not claim production payment security; gateway/webhook verification remains future work.

## Performance Findings

- Settlement adds one indexed lookup by `usage_entry_id` in the transaction path.
- The new partial unique index supports the lookup and conflict boundary.
- No request-path heavy computation or unbounded in-memory storage was added.

## Tests Added

- Added migration coverage for the new `idx_billing_invoices_unique_usage_entry` index.
- Expanded the ignored PostgreSQL BN-18 flow to check exact duplicate Pix confirmation, conflicting Pix confirmation rejection, and no duplicate Pix ledger lines.
- Expanded the ignored PostgreSQL BN-18 flow to check same-reservation billing duplicate behavior and cross-reservation usage rebilling rejection.

## Validation

Passed:

- `cargo fmt --all --check`
- `cargo test -p burd-control-plane`
- `cargo test --workspace`
- `cargo build --workspace`
- `cargo test -p burd-control-plane -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit`
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored`

The PostgreSQL ignored tests were run against a temporary local Docker Postgres 16 container named `burd-hardening-postgres`, then the container was stopped.

Known warnings:

- `cargo test` and `cargo build` still emit inherited warnings from `third_party/llmfit/llmfit-core` about unused upstream internals. This PR did not modify `third_party/llmfit`.

Tests not executed:

- The agent real hardware integration test remains ignored by design under the normal workspace test run.

## Remaining Risks

- Existing deployed databases with historical duplicate `billing_invoices.usage_entry_id` rows would need cleanup before applying migration `0021`.
- BN-18 still lacks real Pix gateway calls, signed webhook validation, banking payout execution, refunds/disputes workflow, and production reconciliation.
- The current settlement model assumes one BN-15 usage ledger entry belongs to one billable invoice.

## Deferred Items

- Real Pix provider adapter and webhook signature verification.
- External payout execution and reconciliation.
- Refunds, disputes, invoice lifecycle policy, legal/KYC/tax workflows, and checkout UX.

## Recommended Next Hardening PRs

- Audit customer credit ledger idempotency and project quota edge cases under concurrent reservation attempts.
- Audit payout state transitions and reconciliation placeholders before connecting any real banking provider.
- Audit OpenAPI error examples for billing conflict cases once API examples are expanded.