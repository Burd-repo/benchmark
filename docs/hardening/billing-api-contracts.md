# Billing API Contracts Hardening

## Summary

This pass audited the implemented BN-18 HTTP/OpenAPI billing contract. It did not add new billing features, Pix gateway calls, payout execution, webhook ingestion, refunds, disputes, or checkout UI.

## Scope Audited

- OpenAPI components for shared error envelopes
- BN-18 admin billing endpoints
- BN-18 customer billing endpoints
- Idempotency-Key documentation for Pix payment-intent creation
- Billing settlement conflict examples
- Pix confirmation conflict documentation
- Provider payout policy conflict documentation

## Implemented For Real

- Runtime errors use the existing JSON envelope `{ "error": { "code": "conflict", "message": "...", "request_id": "req_example", "retry_after_seconds": null, "details": {} } }`.
- Pix payment-intent creation is idempotency-key protected and returns `idempotency_conflict` for changed-body replay.
- Pix confirmation rejects conflicting provider reference or paid timestamp evidence as `conflict`.
- Billing settlement returns the same invoice for same reservation/usage replay and rejects cross-reservation usage reuse as `conflict`.
- Provider payout creation creates accounting records only and can return `conflict` for KYC/tax/minimum/balance/payout-account policy failures.

## Still Planned, Mock, Or Local

- No real Pix provider API is called.
- No webhook signature verification exists.
- No bank payout execution exists.
- No payout paid/failed/cancelled transition endpoint exists.
- Reconciliation events remain schema placeholders.

## Bugs Found

- OpenAPI documented BN-18 endpoints with terse response descriptions and did not expose the shared error-envelope schema.
- Pix idempotency conflicts and ordinary billing/payout conflicts were both described as generic 409 responses.
- The payout endpoint summary could be read without enough context to know it is only an accounting reservation, not bank execution.

## Bugs Fixed

- Added reusable OpenAPI components for `Idempotency-Key`, `ErrorEnvelope`, and standard error responses.
- Added BN-18-specific examples for insufficient billing balance, already-invoiced usage, and payout policy conflict.
- Expanded BN-18 endpoint descriptions and response maps to include relevant 400, 401, 404, and 409 outcomes.
- Documented that provider payouts do not call a bank or mark funds paid.

## Overengineering Removed

- No code abstraction was added beyond standard OpenAPI components.
- No event bus, listener, adapter layer, or new financial service was introduced.

## Events And Listeners

- No event/listener behavior changed.
- Audit writes remain explicit in the existing transaction paths.

## Migrations And Database

- No migrations were added or modified in this PR.
- Database constraints from earlier BN-18 hardening remain unchanged.

## Security Findings

- The OpenAPI contract now documents the redacted database error shape and shared envelope without exposing tokens, Pix keys, Authorization headers, or raw payment secrets.
- The docs explicitly keep raw Pix key storage and bank payout execution out of scope.

## Performance Findings

- No runtime code path changed.
- No request-path allocation, query, cache, or background loop was added.

## Tests Added

- Added OpenAPI test coverage for the shared error envelope, reusable idempotency parameter, BN-18 idempotency conflict response, Pix confirmation conflict response, billing settlement examples, and payout non-bank-execution wording.
- Updated existing OpenAPI idempotency test to resolve both inline and component `$ref` parameters.

## Tests Executed

Passed:

- `cargo fmt --all --check`
- `cargo test -p burd-control-plane`
- `cargo test --workspace`
- `cargo build --workspace`
- `cargo test -p burd-control-plane -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit`

Known warnings:

- `cargo test` and `cargo build` still emit inherited warnings from `third_party/llmfit/llmfit-core` about unused upstream internals. This PR did not modify `third_party/llmfit`.

## Tests Not Executed And Reason

- `BURD_CONTROL_TEST_DATABASE_URL=... cargo test -p burd-control-plane -- --ignored` was not run because this PR did not change runtime billing logic, migrations, database queries, or PostgreSQL behavior.
- The agent real hardware integration test remains ignored by design under normal workspace and crate test runs.

## Remaining Risks

- The OpenAPI document remains hand-authored JSON and can still drift if future endpoints change without matching tests.
- BN-18 admin payout creation is not idempotency-key protected yet; adding that would be a runtime behavior change and should be a separate focused PR.
- Real gateway/webhook/payout/reconciliation security remains future work.

## Deferred Items

- Add idempotency protection to admin payout creation if operator retry semantics require it.
- Add detailed request/response schemas for every BN-18 success body.
- Add OpenAPI examples for customer scope failures once error examples are broadened across all customer APIs.

## Recommended Next Hardening PRs

- Audit admin authorization boundaries across BN-18 endpoints before exposing them beyond private beta operators.
- Audit refund/dispute placeholders before adding any customer-visible or operator-visible workflow.
- Audit OpenAPI request/response schemas outside BN-18, starting with jobs and scheduler state transitions.