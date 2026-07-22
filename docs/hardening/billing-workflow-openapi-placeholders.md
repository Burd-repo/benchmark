# Billing Workflow And OpenAPI Placeholder Hardening

## Summary

This pass audited three remaining hardening areas from the previous BN-18 recommendations: refund/dispute/reconciliation placeholders, provider payout state boundaries, and OpenAPI request/response schemas for implemented jobs, scheduler leases, and customer reservations. It does not add new product workflows, Pix gateway calls, payout execution, payout transition endpoints, refund/dispute APIs, scheduler daemon behavior, provider runtime execution, or marketplace checkout.

## Scope Audited

- `billing_refunds`, `billing_disputes`, and `billing_reconciliation_events` placeholder tables
- Provider payout status vocabulary and current creation-only behavior
- BN-13 job request/response contracts
- BN-14 scheduler and lease request/response contracts
- BN-17 customer marketplace reservation request/response contracts
- OpenAPI drift between implemented HTTP handlers and protocol structs

## Implemented For Real

- Provider payouts can be created from payable balance as `held` or `approved` accounting records.
- Provider payout creation reserves payable into payout clearing through balanced ledger lines.
- Customer reservations can be created, listed, cancelled, and expired by backend logic.
- Jobs, device pull, provider events, final result metadata, scheduler runs, and lease history exist as backend control-plane contracts.
- Refund, dispute, and reconciliation tables exist as database placeholders only.

## Still Planned, Mock, Or Local

- No refund request API exists.
- No dispute adjudication workflow exists.
- No reconciliation ingestion or automatic matching API exists.
- No payout paid/failed/cancelled transition endpoint exists.
- No bank payout execution, Pix gateway call, webhook signature verification, or checkout UI exists.
- Jobs still do not execute provider-side containers or transfer artifact bytes through a real object-storage data plane.

## Bugs Found

- Refund and dispute placeholder tables lacked basic database constraints for positive amounts, currency format, nonblank reasons, and bounded placeholder statuses.
- Dispute placeholders allowed multiple simultaneously open disputes for the same invoice.
- Reconciliation placeholder status was only nonblank, not bounded to known placeholder lifecycle values.
- OpenAPI listed implemented job, scheduler, lease, and reservation endpoints without request/response schemas tied to `burd-protocol` concepts.
- Provider payout statuses included future `paid`, `failed`, and `cancelled` values at the database level, but OpenAPI did not explicitly distinguish current creation states from future transition states.

## Bugs Fixed

- Added migration `0024_refund_dispute_placeholder_integrity` with refund/dispute/reconciliation placeholder constraints and indexes.
- Added database coverage for invalid refund/dispute/reconciliation rows and duplicate open disputes.
- Added OpenAPI schema components for implemented job, scheduler, lease, and reservation contracts.
- Added OpenAPI request body and response schema refs for implemented job, scheduler, device job, lease, reservation, and payout creation endpoints.
- Added OpenAPI tests that payout transition endpoints remain undocumented until implemented.

## Overengineering Removed

- No service layer, event bus, listener, adapter, or generic workflow engine was added.
- The OpenAPI additions are schema metadata and private helper wiring only; they do not change runtime request paths.

## Events And Listeners

- No event bus or listener behavior changed.
- Existing audit events remain explicit writes in the current transaction paths.

## Migrations And Database

- Added `0024_refund_dispute_placeholder_integrity.sql`.
- The migration adds refund amount/currency/status/reason constraints.
- The migration adds dispute hold amount/currency/status/reason constraints and a unique open-dispute-per-invoice index.
- The migration bounds reconciliation placeholder status values and adds lookup indexes.
- Existing migrations were not edited.

## Security Findings

- Placeholder rows are now less likely to accumulate impossible financial states before any operator/customer workflow is exposed.
- OpenAPI now documents that provider payout `paid`, `failed`, and `cancelled` states are reserved for future transition APIs rather than available runtime actions.
- No raw Pix keys, bearer tokens, API keys, customer workload payloads, or private keys were added to logs or docs.

## Performance Findings

- Runtime request paths are unchanged.
- New indexes improve future placeholder inspection by project/status, provider/status, and open dispute lookup.
- The OpenAPI helper runs only when the API document is generated.

## Tests Added

- Migration coverage for `0024_refund_dispute_placeholder_integrity`.
- PostgreSQL ignored-test coverage for invalid refund/dispute/reconciliation placeholder rows and duplicate open dispute prevention.
- OpenAPI schema coverage for implemented job, scheduler, lease, reservation, and payout contracts.
- OpenAPI coverage that payout transition endpoints are not documented before implementation.

## Tests Executed

Passed:

- `cargo fmt --all --check`
- `cargo test -p burd-control-plane`
- `cargo test --workspace`
- `cargo build --workspace`
- `cargo test -p burd-control-plane -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit`
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored`

The PostgreSQL ignored tests were run against a temporary local Docker Postgres 16 container named `burd-placeholder-hardening-postgres`, then the container was stopped.

Known warnings:

- `cargo test` and `cargo build` still emit inherited warnings from `third_party/llmfit/llmfit-core` about unused upstream internals. This PR did not modify `third_party/llmfit`.

Commands that failed before recovery:

- The first unprivileged `docker exec burd-placeholder-hardening-postgres pg_isready ...` readiness loop failed because the Windows sandbox could not access the Docker Desktop pipe. The same readiness check passed when rerun with elevated tool permission.

## Tests Not Executed And Reason

- The agent real hardware integration test remains ignored by design under the normal workspace and crate test runs.

## Remaining Risks

- Existing databases with invalid refund/dispute/reconciliation placeholder rows would need cleanup before applying migration `0024`.
- The OpenAPI document is still hand-authored and can drift if future handlers are added without tests.
- Payout transition APIs still need a separate conservative design for release, failure, cancellation, reconciliation, and ledger reversal rules.
- Refund/dispute workflows still require product, accounting, customer, and operator policy before implementation.

## Deferred Items

- Refund/dispute APIs and state machines.
- Reconciliation ingestion and matching.
- Payout paid/failed/cancelled transition API.
- Payout release-from-hold worker.
- Detailed schemas for the older BN-01 through BN-12 endpoints.

## Recommended Next Hardening PRs

- Audit payout transition design before adding any paid/failed/cancelled endpoint.
- Audit refund/dispute accounting rules before adding customer-visible actions.
- Continue OpenAPI schema coverage for older identity, session, telemetry, evidence, challenge, verification, network, trust, benchmark, workload, marketplace, observability, and security endpoints.