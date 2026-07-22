# Idempotency And State Transition Audit

## Summary

This hardening pass focused on already implemented backend state transitions. It did not add new product features.

## Scope

- marketplace reservation lifecycle
- marketplace listing availability state after reservation cancellation or expiry
- billing invoice settlement idempotency for `(reservation_id, usage_entry_id)`
- migration coverage for the invoice uniqueness constraint

## Bugs Fixed

- Cancelling a reserved marketplace reservation changed the reservation status but left the marketplace listing stuck as `reserved` until a later sweep recalculated it.
- Expiring stale reservations had the same listing-release gap.
- Concurrent billing settlement for the same reservation and usage pair could rely on the database unique constraint and surface as a database error instead of returning the existing invoice as a duplicate response.

## Behavior After This Pass

- Cancelling or expiring the last active reservation for a listing releases the listing status based on the current session state: `available`, `degraded`, `offline`, or `blocked`.
- Billing invoice insert now uses `ON CONFLICT (reservation_id, usage_entry_id) DO NOTHING` and returns the existing invoice with `duplicate = true` when another transaction already created it.
- Financial ledger lines are only appended when the invoice insert actually wins.

## Tests Added Or Expanded

- Pure unit coverage for listing status after reservation release.
- Migration coverage for `UNIQUE(reservation_id, usage_entry_id)`.
- Ignored PostgreSQL reservation flow now checks cancellation releases the listing and allows a second reservation.
- Ignored PostgreSQL billing flow now checks duplicate settlement returns the same invoice and does not duplicate billing ledger lines.

## Validation

Passed:

- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo build --workspace`
- `cargo test -p burd-control-plane`
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored`
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane customer::tests::postgres_customer_reservation_flow_persists_usage_and_cancellation -- --ignored`

During validation, the first PostgreSQL ignored run failed in the customer reservation cancellation flow. The failure exposed that the listing release query used `FOR UPDATE` on a `LEFT JOIN`; PostgreSQL requires locking the listing table explicitly with `FOR UPDATE OF l`. The query now locks only `marketplace_listings` before checking active reservations.

The workspace run still leaves the existing slow local hardware integration test ignored by design. The control-plane PostgreSQL ignored tests were run against a temporary local Docker Postgres database.