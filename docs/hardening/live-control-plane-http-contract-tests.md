# Live Control-Plane HTTP Contract Tests

## Summary

This hardening pass moved part of the BN-01 through BN-12 contract coverage from static OpenAPI assertions to live Axum/router tests. It did not add product features, new endpoints, or new remote runtime behavior.

## Scope Audited

- `crates/burd-control-plane/src/http.rs`
- `docs/current-state.md`
- Implemented BN-01 through BN-11 control-plane route surface
- BN-12 secure runtime boundary, only to ensure no remote runtime execution endpoint is routed or documented

## Implemented For Real

- Live router smoke coverage for implemented BN-01 through BN-11 paths.
- Live error-envelope coverage for representative admin, device/session, and customer protected routes.
- Live idempotency header validation coverage for mutating admin requests.
- Ignored PostgreSQL live HTTP test for readiness, provider creation, idempotent replay, idempotency conflict, and provider lookup through the router.
- Live negative check that BN-12 runtime execution paths are neither in OpenAPI nor routed by Axum.

## Still Planned, Mock, Or Local

- BN-12 remains agent-local secure runtime planning only.
- Agent-side remote Proof of Capability workload execution is still not implemented.
- Production regional probes, scheduler daemon, paid provider execution, Pix gateway, payouts, and external observability integrations remain outside this scope.

## Bugs Found

- No runtime bug required a code-path fix in this pass.
- The main gap was test coverage: OpenAPI listed contracts, but the live router did not yet have broad regression coverage proving implemented paths were actually routed.

## Bugs Fixed

- Added live route drift tests to catch implemented BN-01 through BN-11 paths returning `404 Not Found` or `405 Method Not Allowed`.
- Added live redacted error-envelope tests for protected routes that should fail closed before database access when credentials are absent.
- Added live idempotency header tests proving an authenticated mutating provider request rejects missing or oversized `Idempotency-Key` before database access.
- Added a PostgreSQL-backed live provider registry test proving readiness and idempotency behavior through HTTP, not only through database helpers.

## Overengineering Removed

- None. The pass added small test helpers only inside the existing `http.rs` test module.

## Events And Listeners

- No event bus, listener, async dispatch, or background event plumbing was changed.

## Migrations And Database

- No migrations were added or edited.
- The new PostgreSQL test creates a unique schema, runs all migrations, verifies readiness, and drops the schema at the end.

## Security Findings

- Protected representative admin, device/session, and customer routes return the BN-00 `unauthorized` envelope without touching an unavailable database when credentials are absent.
- Client-facing database-unavailable envelopes do not echo the configured database URL or password.
- BN-12 remote runtime execution endpoints remain absent from both OpenAPI and the live router.

## Performance Findings

- No runtime performance behavior changed.
- The route smoke test uses one in-memory router instance with an unavailable database URL and does not perform network/database work for protected paths.

## Tests Added

- `live_router_serves_bn01_bn11_contract_paths_and_keeps_bn12_runtime_absent`
- `live_protected_routes_return_redacted_error_envelopes_before_database_access`
- `live_mutating_admin_routes_require_bounded_idempotency_keys_after_auth`
- `live_provider_registry_http_contract_persists_idempotency_and_readiness` (`#[ignore]`, PostgreSQL-backed)

## Tests Executed

- `cargo fmt --all --check` passed.
- `cargo test -p burd-control-plane http --lib` passed: 15 passed, 1 ignored.
- `cargo test -p burd-control-plane` passed: 98 passed, 14 ignored.
- `cargo test --workspace` passed. It emitted the known inherited `third_party/llmfit` warnings.
- `cargo build --workspace` passed. It emitted the known inherited `third_party/llmfit` warnings.
- `cargo test -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit` passed.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored` passed against an isolated local Docker Postgres container: 14 passed.

## Tests Not Executed

- No production probe, scheduler daemon, Pix gateway, payout, external observability, or provider runtime execution tests were run because those integrations are not implemented in this scope.
- No full happy-path live HTTP enrollment/session/evidence/challenge fixture suite was added in this pass; that remains the recommended next focused hardening step.

## Remaining Risks

- The live smoke test verifies routing/status boundaries, not full valid payload examples for every BN-01 through BN-11 endpoint.
- Extractor-level JSON rejection still comes from Axum defaults; a future PR should decide whether to wrap extractor rejections in the BN-00 error envelope.
- Full live HTTP happy-path flows for enrollment, remote sessions, telemetry, evidence, proof challenges, benchmark results, and workload policy sweeps remain candidates for focused fixture-backed tests.

## Deferred Items

- Fixture-backed live examples for enrollment/session/evidence/challenge flows.
- Generated OpenAPI schema/example validation against live responses.
- Full BN-13+ live HTTP audit for jobs, leases, reservations, ledger, billing, and marketplace endpoints.

## Recommended Next Hardening PRs

- Add fixture-backed OpenAPI examples for high-risk provider enrollment, remote session, evidence, and challenge flows.
- Audit BN-13 and later docs/API contracts to ensure jobs, scheduler, marketplace, reservations, ledger, and billing endpoints do not overstate production completeness.
- Add focused live HTTP tests for state transitions and idempotency across jobs, reservations, usage ledger, and billing settlement.