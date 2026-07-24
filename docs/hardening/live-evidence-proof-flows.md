# Live Evidence And Proof Challenge Flows

## Summary

This hardening pass adds PostgreSQL-backed live HTTP contract coverage for BN-05 remote evidence registry and BN-06 active proof challenge response verification. It also extracts the repeated provider enrollment and remote session setup into a focused integration helper inside the HTTP test module.

No product feature, agent daemon, scheduler behavior, marketplace behavior, billing behavior, Pix, payouts, or provider runtime/container execution was added.

## Scope Audited

- `crates/burd-control-plane/src/http.rs`
- `docs/current-state.md`
- Existing live provider/enrollment/session HTTP tests
- Remote evidence submission route
- Evidence list/read routes
- Proof challenge issue/next/submit/read routes

## Implemented For Real

- `LiveHttpFixture` now creates an isolated PostgreSQL schema, creates a provider, issues an enrollment token, signs the backend enrollment nonce with a real Ed25519 key, completes enrollment, starts a remote session, and records a heartbeat so later live HTTP tests begin from a real online session.
- `live_evidence_registry_http_flow_persists_valid_evidence_and_deduplicates` submits a freshly signed report through `/v1/sessions/{session_id}/evidence-records`, verifies backend binding flags, verifies duplicate replay returns the existing evidence record, and checks PostgreSQL evidence/hardware snapshot persistence.
- `live_proof_challenge_http_flow_verifies_signed_response` issues a proof challenge, retrieves it through the authenticated session next-challenge route, submits a signed capability response, verifies the response, reads the challenge back, and checks PostgreSQL proof/verification state persistence.
- `live_enrollment_and_remote_session_http_flow_persists_authoritative_state` now reuses the same setup helper instead of duplicating enrollment/session plumbing.

## Still Planned, Mock, Or Local

- These tests validate backend HTTP and PostgreSQL behavior only; they do not run an agent daemon.
- The proof response uses deterministic test metrics and a signed payload, not a real CUDA/VRAM/GEMM/LLM workload runner.
- WebSocket control-channel streaming is still not exercised here.
- Scheduler, jobs, marketplace enforcement, billing settlement, Pix gateway calls, payouts, and provider runtime execution remain outside this PR.

## Bugs Found

- No runtime bug was found in this pass.
- The main risk was drift: evidence and proof challenge contracts had unit/storage coverage, but not a shared live HTTP flow starting from real enrollment/session credentials.

## Bugs Fixed

- Added live coverage that fails if signed evidence no longer binds provider, device, active key, session fingerprint, report hash, object storage, or deduplication correctly.
- Added live coverage that fails if proof challenge issuance, session pickup, signed response verification, metric policy checks, object persistence, or verification-state persistence drift apart.
- Removed duplicated enrollment/session setup from the live HTTP tests by extracting a small test helper.

## Overengineering Removed

- Removed duplicated setup code from the enrollment/session live test and centralized it in `LiveHttpFixture` plus small helper functions.
- No production abstraction or service layer was added.

## Events And Listeners

- No event bus, listener, async dispatch, or background event plumbing changed.

## Migrations And Database

- No migrations were added or edited.
- The live evidence flow verifies existing `evidence_records` and `hardware_snapshots` persistence.
- The live proof flow verifies existing `proof_challenges` and `provider_verification_states` persistence.

## Security Findings

- The live flows use backend-issued enrollment tokens, Ed25519 nonce proofs, backend-issued device credentials, session resume tokens, and `X-Burd-Device-Id` session binding.
- Signed evidence uses the enrolled active key and validates backend binding flags.
- Signed proof responses use the enrolled active key and validate response hash/signature binding.
- No private key, device credential, resume token, database URL, Pix key, or production secret was added to repository fixtures or docs.

## Performance Findings

- No runtime performance behavior changed.
- The new tests are ignored by default and only run when PostgreSQL integration tests are explicitly enabled.

## Tests Added

- `live_evidence_registry_http_flow_persists_valid_evidence_and_deduplicates`
- `live_proof_challenge_http_flow_verifies_signed_response`

## Tests Changed

- `live_enrollment_and_remote_session_http_flow_persists_authoritative_state` now uses the shared live setup helper.

## Tests Executed

- `cargo test -p burd-control-plane` passed: 100 passed, 17 ignored.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored` passed against an isolated Docker PostgreSQL container: 17 passed.
- `cargo fmt --all --check` passed.
- `cargo test --workspace --quiet` passed.
- `cargo build --workspace --quiet` passed.
- `cargo test -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit --quiet` passed.

## Tests Not Executed

- No production deployment, WebSocket control-channel streaming, agent daemon, real CUDA/VRAM/GEMM/LLM proof runner, scheduler daemon, provider runtime/container execution, Pix, payout, or external observability tests were executed.

## Remaining Risks

- The helper drops its temporary schema only on successful test completion; a panic can leave a disposable test schema behind in the isolated database.
- The proof challenge response is structurally and cryptographically valid, but its metrics are deterministic test data rather than measured GPU work.
- Evidence and proof live flows are HTTP fallback/route tests, not full agent daemon end-to-end tests.

## Deferred Items

- Live WebSocket control-channel test for session readiness, server messages, and revocation.
- Live GPU telemetry plus evidence/proof linkage using a shared telemetry window hash.
- Agent-side remote proof workload runner coverage once the agent daemon exists.
- A separate integration-test fixture module if more live HTTP tests reuse `LiveHttpFixture` outside `http.rs`.

## Recommended Next Hardening PRs

- Add a live WebSocket control-channel contract test.
- Add live telemetry-to-proof linkage coverage once a real telemetry window can be produced by test setup.
- Audit whether `LiveHttpFixture` should move into a dedicated integration support module after one more reuse.