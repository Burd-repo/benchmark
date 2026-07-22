# OpenAPI BN-01 to BN-12 Contract Hardening

## Summary

This hardening pass stabilized the control-plane OpenAPI document for the implemented BN-01 through BN-11 backend surfaces and kept BN-12 correctly documented as agent-local secure runtime planning. It did not add product features or new runtime behavior.

## Scope Audited

- `crates/burd-control-plane/src/openapi.rs`
- `docs/current-state.md`
- `docs/remote-protocol-v1.md`
- Implemented BN-01 through BN-11 HTTP endpoints and their Rust protocol/control-plane DTO boundaries
- BN-12 runtime planning status, only to avoid implying a remote execution API exists

## Implemented For Real

- BN-01 health, readiness, provider registry, idempotency, and error envelope foundations.
- BN-02 enrollment token, enrollment proof, device credential, key rotation, device listing, and revocation endpoints.
- BN-03 remote session creation/resume, heartbeat HTTP fallback, session lookup, and revocation endpoints.
- BN-04 signed GPU telemetry batch ingestion and latest telemetry lookup.
- BN-05 signed evidence submission, listing, retrieval, freshness, and revocation.
- BN-06 active proof challenge issuance, retrieval, pickup, and signed response submission.
- BN-07 admin-triggered recurring verification sweep and verification-state listing.
- BN-08 trusted network probe observation ingestion and provider network state listing.
- BN-09 admin-triggered trust sweep, trust-state listing, and antifraud event listing.
- BN-10 benchmark profile upsert/list and signed benchmark result submit/list.
- BN-11 workload policy upsert/list and backend-derived workload eligibility sweep/list.
- BN-12 local secure runtime checks/plans in the agent/bench crates, with no remote runtime execution endpoint.

## Still Planned, Mock, Or Local

- Agent-side remote Proof of Capability workload execution remains unimplemented.
- Production regional probe workers remain undeployed.
- Secure provider runtime execution for paid jobs remains unimplemented.
- Scheduler/job/marketplace/billing features after BN-12 are outside this OpenAPI hardening scope.
- Full generated deep OpenAPI schemas for every nested Rust serde field are still deferred; the new schemas are structural top-level contracts with required fields and component refs.

## Bugs Found

- Many implemented BN-01 through BN-11 endpoints were listed in OpenAPI without request body schema refs or JSON response schema refs.
- Telemetry and heartbeat HTTP fallbacks could be misread as payload-only APIs unless documented as control-message envelopes.
- BN-12 could be misinterpreted as having a remote runtime execution API if the docs did not call out its agent-local boundary.

## Bugs Fixed

- Added structural component schemas for implemented BN-01 through BN-11 request and response envelopes.
- Attached request body schema refs to implemented mutating endpoints.
- Attached JSON response schema refs to implemented successful responses, including duplicate/idempotent `200` variants where the runtime returns them.
- Fixed the new OpenAPI regression test after it initially expected `201/202` for routes whose implementation intentionally returns `200 OK` (`complete_key_rotation`, heartbeat, and telemetry HTTP fallback).
- Preserved the `ClientControlMessage` boundary by documenting heartbeat and telemetry HTTP fallback request bodies as envelope-specific schemas.
- Added regression tests for BN-01 through BN-11 schema presence and endpoint request/response refs.

## Overengineering Removed

- None. This pass avoided restructuring the OpenAPI document generator beyond a small structural schema helper.

## Events And Listeners

- No event bus, listener, async dispatch, or background event plumbing was changed.

## Migrations And Database

- No migrations were added or edited.
- No database schema or persistence behavior changed.

## Security Findings

- The OpenAPI contract now better reflects bearer-protected device/admin boundaries already present in the route descriptions and existing tests.
- The telemetry and heartbeat HTTP fallback schemas avoid implying that unsigned or unsequenced payloads can be submitted without the control-message envelope.
- No secrets, credentials, or Authorization headers were added to docs, schemas, or logs.

## Performance Findings

- No runtime performance issue was changed in this pass.
- The OpenAPI additions are static document construction at request time, consistent with the existing implementation style.

## Tests Added

- `openapi_documents_bn01_bn11_request_response_schemas` in `crates/burd-control-plane/src/openapi.rs`.

## Tests Executed

- `cargo test -p burd-control-plane openapi --lib` passed after aligning the new regression test with existing runtime status codes.
- `cargo fmt --all --check` passed.
- `cargo test -p burd-control-plane` passed: 95 passed, 13 ignored.
- `cargo test --workspace` passed. It emitted the known inherited `third_party/llmfit` warnings.
- `cargo build --workspace` passed. It emitted the known inherited `third_party/llmfit` warnings.
- `cargo test -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit` passed.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored` passed against an isolated local Docker Postgres container: 13 passed.

## Tests Not Executed

- No BN-12 remote runtime execution test was run because BN-12 has no remote execution endpoint in this repository.
- No production probe, scheduler daemon, Pix gateway, payout, or external observability tests were run because those integrations are not implemented in this scope.

## Remaining Risks

- The schemas are structural top-level contracts, not exhaustive generated schemas for every nested field.
- OpenAPI is still manually maintained, so future endpoint changes can drift unless tests are extended with each BN.
- BN-12 remains local runtime planning only; provider-side job execution and sandbox orchestration still need their own later contracts.

## Deferred Items

- Generated OpenAPI schemas from Rust types.
- Example request/response payload fixtures for every BN endpoint.
- Full contract tests that compare live HTTP responses from an isolated test server against OpenAPI examples.

## Recommended Next Hardening PRs

- Add live control-plane HTTP contract tests for auth, status codes, and JSON envelopes across BN-01 through BN-12 surfaces.
- Add generated or fixture-backed OpenAPI examples for high-risk provider/session/evidence/challenge flows.
- Audit docs for BN-13 and later to ensure jobs, scheduler, marketplace, reservations, ledger, and billing endpoints do not overstate production completeness.