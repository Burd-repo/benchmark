# Provider Payout Reconciliation Hardening

## Summary

This pass audited the implemented BN-18 provider payout and reconciliation primitives. It did not add real Pix payout execution, banking API calls, webhook ingestion, payout release automation, refunds, disputes, or marketplace checkout flows.

## Scope Audited

- provider payout account validation and persistence
- provider payout creation from payable balance
- financial ledger movement into provider payout clearing
- database invariants around payout status and reconciliation placeholders
- BN-18/current-state documentation claims

## Implemented For Real

- Admins can upsert Pix payout accounts that store hash material and masked suffixes, not raw Pix keys.
- Provider payout creation requires an active payout account, verified KYC/tax status, minimum payout, sufficient provider payable balance, and the requested currency.
- Payout creation reserves provider payable into `provider_payout_clearing` through balanced ledger lines.
- Payout hold days create `held` payouts; zero hold days create `approved` payouts.
- `billing_reconciliation_events` exists as a database placeholder with uniqueness by provider, external reference, and event type.

## Still Planned, Mock, Or Local

- No bank payout is executed.
- No payout is marked paid by an adapter or webhook.
- No reconciliation event ingestion API exists.
- No release-from-hold worker exists.
- Refunds, disputes, tax/KYC/legal workflows, and checkout remain future work.

## Bugs Found

- Database writes outside the control-plane API could insert invalid financial ledger rows, payout account states, payout states, held payouts without `hold_until`, paid payouts without a reference, or blank/zero reconciliation placeholder data.
- The existing PostgreSQL billing flow only covered zero-hold approved payouts and did not verify held payout accounting or payout clearing balances.

## Bugs Fixed

- Added database constraints for non-zero financial ledger amounts, positive ledger line numbers, and ledger currency format.
- Added database constraints for payout account method, currency, KYC/tax status, minimum payout, hold-day range, and account status.
- Added database constraints for payout status, positive amount, currency, held payout `hold_until`, nonblank external references, and paid payout reference/timestamp requirements.
- Added basic reconciliation placeholder constraints for positive amount, currency format, and nonblank provider/reference/event/status fields.
- Added a unique index for non-null payout external references and a lookup index for reconciliation provider/reference inspection.

## Overengineering Removed

- No abstraction was added or removed. The hardening stays at the database boundary and in focused tests.
- No event bus, listener layer, or async dispatch path was introduced.

## Events And Listeners

- No event bus or listener plumbing was changed.
- Audit events for payout account upsert and payout creation remain explicit writes inside the existing transaction.

## Migrations And Database

- Added `0023_provider_payout_reconciliation_integrity.sql`.
- Existing migrations were not edited.
- The migration is additive and uses guarded constraint creation so a partially applied local schema can be inspected safely.
- The migration validates existing rows when constraints are added; deployed databases with invalid historical rows need cleanup before applying it.

## Security Findings

- Payout records now have database-level guards against impossible paid/held states if future adapter code bypasses normal helpers.
- Ledger rows now reject zero-amount direct inserts.
- The change does not log private keys, tokens, authorization headers, raw Pix keys, or payment secrets.
- This PR does not claim production payment or payout security.

## Performance Findings

- Added a partial unique index on payout external references for future reconciliation collision prevention.
- Added an index on reconciliation provider/reference/time for future manual inspection or adapter matching.
- No request-path heavy computation, unbounded cache, or new background loop was added.

## Tests Added

- Added migration coverage for the new provider payout/reconciliation integrity migration.
- Expanded the ignored PostgreSQL BN-18 flow to reject invalid direct ledger, payout account, payout, and reconciliation inserts.
- Expanded the ignored PostgreSQL BN-18 flow to cover held payout creation, approved payout creation after account hold change, provider payable reduction, payout ledger lines, and payout clearing balance.

## Tests Executed

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

## Tests Not Executed And Reason

- The agent real hardware integration test remains ignored by design under normal workspace and crate test runs.

## Remaining Risks

- There is still no payout execution adapter, signed webhook validation, external reconciliation ingestion, or release-from-hold workflow.
- Existing databases with invalid direct financial rows would need manual cleanup before migration `0023` can apply.
- Payout external reference uniqueness is global because the current payout table does not store a banking provider namespace.

## Deferred Items

- Real payout execution adapter.
- Reconciliation ingestion and matching workflow.
- Payout paid/failed/cancelled transition API.
- Refund and dispute state machines.
- KYC/tax/legal workflow integration.

## Recommended Next Hardening PRs

- Audit OpenAPI examples and response contracts for BN-18 billing conflict cases.
- Audit admin authorization boundaries across all BN-18 endpoints before exposing them beyond local/private beta operators.
- Audit refund/dispute placeholders before adding any customer-visible flow.