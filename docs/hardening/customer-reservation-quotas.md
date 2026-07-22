# Customer Reservation Quotas Hardening

## Summary

This pass audited the implemented BN-17 customer reservation, quota, and non-settlement credit ledger primitives. It did not add checkout, customer job submission, marketplace UI, billing, Pix, payouts, or provider-side execution.

## Scope Audited

- customer credit grant retry behavior
- reservation hold/release ledger markers
- reservation expiration side effects
- project quota enforcement path for reservations
- BN-17 migration constraints and API docs

## Implemented For Real

- Customer projects have backend-owned active reservation and reserved GPU-second quotas.
- Reservation creation is idempotency-key protected and serialized through the project/quota lock path.
- Active marketplace reservations are unique per listing.
- Customer credit ledger entries are append-only and used for non-settlement grants, adjustments, reservation holds, and releases.

## Still Planned, Mock, Or Local

- BN-17 credits are not financial settlement and are not customer cash balance.
- Reservations do not submit jobs, run provider containers, enforce checkout, charge money, or create invoices.
- Scheduler-driven customer demand matching remains future work.

## Bugs Found

- Admin customer credit grants appended a new credit ledger entry on every retry because the endpoint did not use the idempotency table.
- Reservation expiration changed reservation state and released the marketplace listing, but did not append the zero-value `reservation_release` credit ledger marker that explicit cancellation writes.
- The database did not enforce one reservation hold/release marker per reservation and marker type.

## Bugs Fixed

- Customer credit grants now require and store `Idempotency-Key` through the same request-hash replay/conflict model used by other mutating backend operations.
- Expiring reserved reservations now appends a zero-value `reservation_release` ledger entry before releasing the listing hold.
- Added a partial unique index preventing duplicate `reservation_hold` or `reservation_release` entries for the same reservation.

## Overengineering Removed

- No new abstraction layer, event bus, listener, or scheduler was introduced.
- The fixes stay in explicit database transactions and small helper paths.

## Events And Listeners

- No event bus or listener plumbing was changed.
- Customer audit events and global audit events remain explicit writes in the transaction.

## Migrations And Database

- Added `0022_unique_customer_reservation_credit_entries.sql`.
- Existing migrations were not edited.
- The new index applies only when `reservation_id IS NOT NULL` and `entry_type` is `reservation_hold` or `reservation_release`.

## Security Findings

- Idempotent credit grants reduce the risk of duplicated admin accounting entries caused by retries.
- No customer API key tokens, bearer tokens, or raw secrets are logged or stored in new response paths.
- This remains a non-settlement credit ledger; it is not a production payment balance.

## Performance Findings

- Credit grant idempotency adds one bounded idempotency-table lookup on duplicate/retry paths.
- Reservation expiration adds one append-only ledger insert per reservation actually transitioned from `reserved` to `expired`.
- No unbounded in-memory queues, background workers, or heavy request-path computation were added.

## Tests Added

- Added migration coverage for the reservation credit marker uniqueness index.
- Expanded the ignored PostgreSQL customer reservation flow to verify idempotent credit grant replay, idempotency conflict on changed request hash, and no duplicate credit grant ledger entries.
- Expanded the ignored PostgreSQL customer reservation flow to verify expired reservations append exactly one `reservation_release` marker and release the listing.

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

- Existing deployed databases with duplicate reservation hold/release markers would need cleanup before applying migration `0022`.
- Reservation lead-time policy is still minimal; future reservations can be represented, but there is no separate maximum start lead-time policy yet.
- Customer credit ledger remains non-settlement accounting and should not be used as production billing balance.

## Deferred Items

- Idempotency for other admin-only customer creation endpoints, if those become public operational APIs.
- Dedicated concurrent reservation stress tests beyond the transaction/constraint-backed PostgreSQL flow.
- Reservation-to-job checkout/orchestration, which belongs to future product work.

## Recommended Next Hardening PRs

- Audit payout state transitions and reconciliation placeholders before connecting any real banking provider.
- Audit OpenAPI examples/error envelopes around billing/customer conflict cases.
- Add a narrow concurrency stress test harness for reservations once CI can run isolated PostgreSQL jobs reliably.