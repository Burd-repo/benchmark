# Jobs, Leases, Scheduler Hardening

## Summary

This pass audited the implemented BN-13/BN-14/BN-15 job, lease, scheduler, metering, and GPU inventory boundaries. It did not add new product features.

## Scope

- backend compute job lifecycle
- scheduler lease assignment checks
- provider job pull/accept/event/result flow
- usage finalization boundary
- multi-GPU inventory records used by jobs and scheduler decisions

## Bugs Found And Corrected

- `device_gpu_inventory.inventory_hash` was unique even though the BN-21 code stores one immutable row per GPU in the same signed snapshot. Multi-GPU snapshots would fail after the first row.
- Job creation and scheduler eligibility checked whether a GPU had ever had an active inventory row, not whether the latest row for that GPU was active.
- Inventory resubmission deduplication relied on a pre-insert lookup only; concurrent duplicate submissions could surface a database uniqueness error once per-GPU uniqueness is enforced.

## Behavior After This Pass

- A new migration drops the single-column `inventory_hash` uniqueness constraint and replaces it with per-snapshot/per-GPU uniqueness on `(inventory_hash, gpu_index)`.
- Device GPU inventory insert uses `ON CONFLICT (inventory_hash, gpu_index) DO NOTHING`, keeping duplicate submissions idempotent at the persistence boundary.
- Job creation and scheduler candidate selection now require the latest inventory row for the requested GPU UUID to be `active`.
- Older active inventory history no longer keeps a GPU eligible after a newer inactive, degraded, or retired snapshot.

## Overengineering Removed

- No abstraction layer was added. The fix stays in the existing SQL persistence boundary and keeps the explicit job/scheduler checks.

## Events And Listeners

- No event bus or listener plumbing was added or removed.
- Existing audit events for accepted/rejected inventory and lease offers remain explicit database writes in the current transaction.

## Migrations And Database

- Added `0020_gpu_inventory_snapshot_uniqueness.sql`.
- Existing applied migrations were not edited.
- The new migration is deterministic and preserves immutable inventory history while allowing legitimate multi-GPU snapshots.

## Security Findings

- The fix tightens the backend trust boundary: jobs and scheduler decisions no longer trust stale active GPU inventory when a newer snapshot says the GPU is not active.
- No secrets or credentials are logged or exposed by this change.

## Performance Findings

- Latest-inventory checks use the existing provider/device/GPU/time index shape.
- No request-path heavy work was introduced.

## Tests Added Or Adjusted

- Added migration coverage for the per-GPU snapshot uniqueness migration.
- Added ignored PostgreSQL coverage that inserts multiple GPU rows for one snapshot hash.
- Added ignored PostgreSQL coverage that verifies latest GPU inventory status gates job eligibility checks.

## Validation

Passed:

- `cargo fmt --all --check`
- `cargo test -p burd-control-plane`
- `cargo test --workspace`
- `cargo build --workspace`
- `cargo test -p burd-control-plane -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit`
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored`

The first PostgreSQL ignored run failed because `db::tests::migrates_and_persists_provider_transactionally` had a hardcoded expected migration list ending at `0019`. The test now derives expected versions from `MIGRATIONS`, and the rerun passed.

Known warnings:

- `cargo test` and `cargo build` still emit inherited warnings from `third_party/llmfit/llmfit-core` about unused upstream internals. This PR did not modify `third_party/llmfit`.

Tests not executed:

- The agent real hardware integration test remains ignored by design under the normal workspace test run.

## Remaining Risks

- BN-14 still schedules only already-created, already-targeted jobs; it is not a full marketplace demand scheduler.
- BN-21 remains an inventory foundation, not distributed multi-provider placement.
- Provider-side job execution and sandboxed data-plane transfer remain separate implementation work.