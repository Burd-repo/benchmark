# Billing Authorization Boundaries Hardening

## Summary

This pass audited the implemented BN-18 authorization boundary between customer project billing endpoints and admin/operator billing endpoints. It did not add new billing features, Pix gateway calls, payout execution, webhook ingestion, refunds, disputes, checkout UI, or scheduler behavior.

## Scope Audited

- BN-18 customer billing endpoints
- BN-18 admin billing, price-book, provider ledger, payout account, and payout endpoints
- Customer billing scopes
- OpenAPI security schemes for BN-18 endpoints
- Idempotency-Key documentation on BN-18 billing routes
- Current protocol and state documentation around customer/admin credential separation

## Implemented For Real

- Customer API keys authenticate project-scoped billing endpoints.
- `billing:write` is required for creating Pix payment intents.
- `billing:read` is required for project balance and project financial ledger reads.
- Admin bearer authorization gates listing price writes, Pix confirmation, reservation settlement, invoice reads, provider balance/ledger reads, payout account upsert, and payout creation.
- Provider/device session credentials are separate from customer/admin billing credentials.

## Still Planned, Mock, Or Local

- No real Pix gateway call exists.
- No webhook signature verification exists.
- No bank payout execution exists.
- No production KYC/tax workflow exists.
- No checkout UI, refund workflow, dispute workflow, or automated reconciliation ingestion exists.

## Bugs Found

- OpenAPI described Pix payment-intent creation as requiring `Idempotency-Key`, but did not explicitly document the required `billing:write` customer scope.
- There was no focused test preventing BN-18 admin endpoints from being documented with customer auth or BN-18 customer endpoints from being documented with admin auth.
- Billing scope tests did not explicitly prove that `billing:read`, `billing:write`, and unrelated reservation scopes are not interchangeable.

## Bugs Fixed

- Pix payment-intent OpenAPI description now documents the required `billing:write` scope.
- Added OpenAPI coverage that BN-18 admin endpoints use only `adminBearer` and do not document `Idempotency-Key` unless runtime requires it.
- Added OpenAPI coverage that BN-18 customer billing endpoints use only `customerBearer`, with `billing:read`/`billing:write` documented on the relevant operations.
- Added a billing unit test proving customer billing scopes are exact, not interchangeable.

## Overengineering Removed

- No abstraction, service layer, event bus, listener, middleware, or adapter was added.
- The existing explicit HTTP authorization and database scope checks were kept because they protect real admin/customer boundaries.

## Events And Listeners

- No event bus or listener behavior changed.
- Billing audit and ledger writes remain explicit in existing transaction paths.

## Migrations And Database

- No migrations were added or modified.
- No schema, index, trigger, foreign key, ledger, reservation, payout, or idempotency table behavior changed.

## Security Findings

- The runtime already separates admin bearer authorization from customer API-key authorization for the audited BN-18 routes.
- OpenAPI now has regression coverage for that split so generated docs do not accidentally advertise customer access to admin billing/provider payout endpoints.
- No private keys, API keys, bearer tokens, Authorization headers, Pix key material, or payment secrets are logged or added to docs.

## Performance Findings

- No runtime path changed.
- The added tests and documentation do not add request-path CPU, database queries, locks, caches, or background work.

## Tests Added

- `billing_customer_scopes_are_not_interchangeable`
- `openapi_documents_bn18_admin_customer_authorization_boundaries`

## Tests Executed

Passed:

- `cargo fmt --all --check`
- `cargo test -p burd-control-plane`
- `cargo test --workspace`
- `cargo build --workspace`
- `cargo test -p burd-control-plane -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit`
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored`

The PostgreSQL ignored tests were run against a temporary local Docker Postgres 16 container named `burd-billing-auth-postgres`, then the container was stopped. An earlier nested PowerShell invocation of the same ignored-test command failed because `$env:BURD_CONTROL_TEST_DATABASE_URL` was stripped by quoting before `cargo test` started; rerunning with the env var set in the current shell passed.

Known warnings:

- `cargo test` and `cargo build` still emit inherited warnings from `third_party/llmfit/llmfit-core` about unused upstream internals. This PR did not modify `third_party/llmfit`.

## Tests Not Executed And Reason

- The agent real hardware integration test remains ignored by design under the normal workspace and crate test runs.

## Remaining Risks

- BN-18 admin payout creation is still not idempotency-key protected; adding that would be a runtime behavior change and should be handled in a separate focused PR.
- Customer/admin authorization is still implemented with the current private-beta credential model, not a full production RBAC system.
- Real Pix gateway, webhook, payout execution, refund/dispute, reconciliation, and compliance workflows remain future work.

## Deferred Items

- Add idempotency protection to admin payout creation if operator retry semantics require it.
- Add explicit 403 behavior if the wider API decides to distinguish authenticated-but-underscoped keys from missing/invalid credentials.
- Add production RBAC once the operator/customer account model moves beyond private beta.

## Recommended Next Hardening PRs

- Audit refund/dispute/reconciliation placeholders before exposing any operator workflow around them.
- Audit payout state transitions before adding paid/failed/cancelled transition endpoints.
- Audit OpenAPI request/response schemas outside BN-18, starting with jobs, scheduler, and customer reservations.