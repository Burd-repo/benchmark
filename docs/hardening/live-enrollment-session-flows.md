# Live Enrollment And Remote Session Flows

## Summary

This hardening pass adds a PostgreSQL-backed live HTTP contract test for the BN-02 enrollment and BN-03 remote session boundary. It does not add product features, agent daemon behavior, remote Proof of Capability execution, scheduler behavior, marketplace behavior, billing behavior, Pix, payouts, or provider runtime execution.

## Scope Audited

- `crates/burd-control-plane/src/http.rs`
- `docs/current-state.md`
- `docs/remote-protocol-v1.md`
- Provider creation route
- Enrollment token issuance route
- Enrollment start and proof routes
- Device listing route
- Remote session start route
- Remote heartbeat HTTP fallback route
- Remote session read route

## Implemented For Real

- One ignored live HTTP test creates a provider through Axum, issues an enrollment token, starts enrollment, signs the backend nonce with a real Ed25519 key, completes enrollment, lists the enrolled device, starts a remote session with the issued device credential, submits a heartbeat with the authenticated session headers, rejects heartbeat replay, reads the session back, and confirms persisted PostgreSQL state.
- The test uses an isolated PostgreSQL schema and drops it at the end of the successful flow.
- Remote protocol docs now explicitly list the required heartbeat fallback headers.

## Still Planned, Mock, Or Local

- The test validates backend HTTP and PostgreSQL behavior only; it does not run an agent daemon.
- Remote Proof of Capability workload execution remains unimplemented.
- Control-channel WebSocket streaming is not exercised by this test; the HTTP heartbeat fallback is exercised.
- Scheduler, jobs, marketplace enforcement, billing settlement, Pix gateway calls, payouts, and provider runtime execution remain outside this PR.

## Bugs Found

- The live test initially failed because the heartbeat request omitted `X-Burd-Device-Id`; the backend correctly rejected it with `401` before database mutation. The test and protocol documentation now exercise the real session header contract.

## Bugs Fixed

- Added live coverage that fails if enrollment proof, issued device credentials, session resume token, device ID binding, heartbeat sequencing, or persisted session state drift apart.
- Documented the heartbeat HTTP fallback headers explicitly in `docs/remote-protocol-v1.md`.

## Overengineering Removed

- None. This PR adds focused test coverage and documentation only.

## Events And Listeners

- No event bus, listener, async dispatch, or background event plumbing changed.

## Migrations And Database

- No migrations were added or edited.
- The live test verifies existing PostgreSQL persistence across providers, devices, provider sessions, and session heartbeats.

## Security Findings

- The live flow uses the real backend-issued enrollment token, Ed25519 nonce proof, backend-issued device credential, session resume token, and device ID header.
- The test confirms stale heartbeat sequence replay is rejected with a conflict envelope.
- No private key, device credential, resume token, database URL, Pix key, or production secret was added to repository fixtures or docs.

## Performance Findings

- No runtime performance behavior changed.
- The new test is ignored by default and only runs when PostgreSQL integration tests are explicitly enabled.

## Tests Added

- `live_enrollment_and_remote_session_http_flow_persists_authoritative_state`

## Tests Executed

- `cargo test -p burd-control-plane` passed: 100 passed, 15 ignored.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored` initially failed on the new heartbeat request because `X-Burd-Device-Id` was missing.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored` passed after the header-contract fix: 15 passed.
- `cargo fmt --all --check` passed.
- `cargo test --workspace --quiet` passed.
- `cargo build --workspace --quiet` passed.
- `cargo test -p burd-agent -p burd-api-local -p burd-bench -p burd-protocol -p burd-hardware -p burd-llmfit --quiet` passed.

## Tests Not Executed

- No production deployment, WebSocket control-channel streaming, agent daemon, remote Proof of Capability runner, scheduler daemon, provider runtime/container execution, Pix, payout, or external observability tests were executed because this PR only hardens backend HTTP contract coverage for enrollment and remote session flows.

## Remaining Risks

- The HTTP flow validates enrollment and heartbeat fallback, but not the long-lived WebSocket control channel.
- The test does not execute GPU telemetry, evidence submission, proof challenge verification, job assignment, scheduler leases, marketplace reservations, billing, or runtime sandbox flows.
- The successful test path drops its temporary schema; a panic before cleanup can leave a disposable test schema behind in the isolated database.

## Deferred Items

- Live HTTP evidence submission using the enrolled device/session produced by a shared fixture helper.
- Live HTTP proof challenge issuance and signed response submission.
- Live WebSocket control-channel contract test with session readiness and server revocation messages.
- Reusable integration fixture helper once the third or fourth live HTTP flow would otherwise duplicate setup.

## Recommended Next Hardening PRs

- Add live HTTP evidence registry flow using a freshly enrolled device and session.
- Add live HTTP proof challenge flow that signs and verifies a minimal valid capability response.
- Add a small integration-test fixture module if those next flows duplicate the enrollment/session setup too heavily.