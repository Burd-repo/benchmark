# Control-Plane Protocol Fixtures And Examples

## Summary

This hardening pass added fixture-backed OpenAPI examples for the highest-risk BN-02 through BN-06 control-plane flows. It did not add product features, endpoints, authentication behavior, scheduler behavior, marketplace behavior, billing behavior, Pix, payouts, or provider runtime execution.

## Scope Audited

- `crates/burd-control-plane/src/openapi.rs`
- `docs/examples/control-plane/*.json`
- `docs/current-state.md`
- Enrollment start/proof contracts
- Remote session start and heartbeat HTTP fallback contracts
- Remote evidence submission contracts
- Proof challenge issuance and signed response contracts

## Implemented For Real

- Static JSON fixtures under `docs/examples/control-plane` for request and response examples.
- OpenAPI `components.examples` entries backed by the same JSON files through `include_str!`.
- Request-body examples wired to enrollment, session, heartbeat, evidence, challenge issuance, and proof-response endpoints.
- Successful response examples wired to the matching implemented 2xx response contracts.
- Tests proving every fixture parses into its corresponding `burd-protocol` Rust type.
- Tests proving OpenAPI request/response examples point to the expected component examples.

## Still Planned, Mock, Or Local

- The examples use redacted placeholder tokens, signatures, and hashes; they are contract examples, not valid cryptographic proofs.
- At the time of this pass, Agent-side remote Proof of Capability workload execution remained unimplemented; the later BN-06 Agent runner adds it.
- Full live HTTP happy-path fixtures for enrollment, session, evidence, and proof challenge verification remain future work.
- BN-12 secure runtime remains local planning only, with no remote execution endpoint added.

## Bugs Found

- No runtime behavior bug was found in this pass.
- The gap was documentation/test drift risk: high-risk protocol examples were not tied to the actual Rust serde contracts.

## Bugs Fixed

- Added parseable protocol fixtures for enrollment, remote session, heartbeat, evidence, and proof challenge examples.
- Wired those fixtures into OpenAPI request and response examples.
- Added OpenAPI tests that fail if examples drift away from `burd-protocol` structs.
- Added a guard that protocol examples do not contain obvious secret material such as `private_key`, `password`, or database URLs.

## Overengineering Removed

- None. The implementation uses small static fixtures and local OpenAPI helper functions.

## Events And Listeners

- No event bus, listener, async dispatch, or background event plumbing was changed.

## Migrations And Database

- No migrations were added or edited.
- No database schema, query, transaction, or persistence behavior changed.

## Security Findings

- Example credentials, tokens, signatures, hashes, and resume tokens are explicit redacted placeholders.
- No Authorization header, private key, password, Pix key, database URL, or production secret was added to the fixtures.
- The examples preserve the device control-message envelope for heartbeat instead of documenting a raw payload-only API.

## Performance Findings

- No runtime performance behavior changed.
- The fixtures are parsed only when generating the OpenAPI document or running OpenAPI tests.

## Tests Added

- `openapi_protocol_examples_parse_into_burd_protocol_contracts`
- `openapi_wires_protocol_examples_to_high_risk_request_and_response_contracts`

## Tests Executed

- `cargo fmt --all --check` passed.
- `cargo test -p burd-control-plane` passed: 100 passed, 14 ignored.
- `cargo test --workspace --quiet` passed.
- `cargo build --workspace --quiet` passed.
- `cargo test -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit --quiet` passed.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored` passed against an isolated Docker PostgreSQL container: 14 passed.

## Tests Not Executed

- No production control-plane deployment, scheduler daemon, provider runtime/container execution, Pix, payout, or external observability tests were executed because this PR only changes static protocol examples, OpenAPI wiring, and documentation.
- The redacted proof/evidence fixtures were parsed structurally but were not verified as cryptographically valid signed payloads.

## Remaining Risks

- The examples are structural and parseable but not cryptographically valid signed payloads.
- The OpenAPI schemas remain manually authored structural schemas rather than generated deep schemas.
- Full valid live HTTP flows still need backend-seeded fixtures and signing helpers if we want end-to-end success examples.

## Deferred Items

- Live HTTP happy-path fixture tests for enrollment, remote session, signed evidence, and proof challenge verification.
- Generated OpenAPI schemas or a schema-generation strategy tied directly to Rust types.
- BN-13+ examples for jobs, leases, reservations, usage ledger, billing, and marketplace contracts.

## Recommended Next Hardening PRs

- Add focused live HTTP happy-path fixtures for enrollment and remote session using an isolated PostgreSQL schema.
- Audit BN-13 and later docs/API contracts so jobs, scheduler, marketplace, reservations, ledger, and billing endpoints do not overstate production completeness.
- Add fixture-backed examples for jobs, reservations, usage ledger, and billing settlement once BN-13+ documentation is re-audited.