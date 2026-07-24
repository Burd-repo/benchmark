# Live Control Channel WebSocket Contract

## Summary

This hardening pass adds PostgreSQL-backed live WebSocket contract coverage for the BN-03 remote session control channel. The test starts from real backend enrollment, opens an authenticated outbound-style control channel against an actual Axum TCP listener, validates server readiness, rejects a duplicate channel, acknowledges a heartbeat, and delivers server-side revocation over the open socket.

No agent daemon, job runtime, scheduler daemon, Proof of Capability workload runner, marketplace feature, Pix gateway, billing automation, or payout flow was added.

## Scope Audited

- `crates/burd-control-plane/src/http.rs`
- `crates/burd-control-plane/src/remote_session.rs`
- `crates/burd-control-plane/Cargo.toml`
- Existing live provider, enrollment, session, evidence, and proof HTTP fixtures
- BN-03 control channel route: `GET /v1/sessions/{session_id}/control`
- BN-03 revocation route: `POST /v1/sessions/{session_id}/revoke`

## Implemented For Real

- The control plane now has a live ignored test that binds an Axum server to `127.0.0.1:0` and performs a real WebSocket upgrade with session headers.
- The live fixture can create either an online session with an initial HTTP heartbeat or a pending session suitable for WebSocket connection tests.
- The WebSocket test validates `session_ready` with backend heartbeat policy values.
- The WebSocket test verifies the backend persists the connected session as `online` with `sequence_last = 0` before the first control-channel heartbeat.
- The WebSocket test verifies a second simultaneous control channel for the same session fails to upgrade.
- The WebSocket test sends a real JSON heartbeat message over the socket and validates `heartbeat_ack` plus sequence acknowledgement.
- The WebSocket test revokes the session through the admin HTTP route and validates `session_revoked` is pushed through the existing channel registry.

## Still Planned, Mock, Or Local

- The test uses a synthetic test agent identity and deterministic fixture payloads; it does not launch a long-running Burd Agent daemon.
- The heartbeat payload is a protocol fixture, not a real local hardware scan.
- The test does not stream telemetry batches, benchmark data, job events, or proof responses over WebSocket.
- Remote Proof of Capability execution, secure runtime execution, scheduler loops, marketplace enforcement, Pix gateway calls, billing automation, and payouts remain outside this hardening pass.

## Bugs Found

- No production bug was found in this pass.
- The coverage gap was that the BN-03 control-channel route previously had unit-level registry coverage and HTTP session coverage, but not a live WebSocket handshake/message contract against the router.

## Bugs Fixed

- Added a regression test that fails if the WebSocket route stops requiring real session authorization headers.
- Added a regression test that fails if duplicate active control channels are accidentally allowed.
- Added a regression test that fails if server-side revocation stops notifying the connected provider channel.
- Added a pending-session fixture path so WebSocket tests do not depend on an HTTP heartbeat making the session already `online` before upgrade.

## Overengineering Removed

- No production abstraction was added.
- The existing live HTTP fixture was extended with one boolean setup path instead of creating a separate enrollment/session fixture stack.

## Events And Listeners

- No event bus, listener, async dispatch, background event, or outbox behavior changed.
- The test covers the existing `ControlChannelRegistry` revocation notification path through a real HTTP admin call and open WebSocket connection.

## Migrations And Database

- No migrations were added or edited.
- The live test uses isolated PostgreSQL schemas through the existing test database fixture.
- The test verifies session status and sequence persistence through the API after the WebSocket connects.

## Security Findings

- The WebSocket upgrade requires the same credential, resume token, and device ID binding as other protected session routes.
- Duplicate channel rejection happens before a second channel can take ownership of the session.
- No private key, raw credential, resume token, admin token, or database URL was committed.

## Performance Findings

- No runtime performance behavior changed.
- The new WebSocket coverage is ignored by default and only runs with explicit PostgreSQL integration testing.
- The test binds to an ephemeral local port and shuts the Axum server down after completion.

## Tests Added

- `http::tests::live_control_channel_websocket_flow_acknowledges_duplicate_and_revocation`

## Tests Changed

- `live_enrolled_session_fixture` now delegates to `live_enrolled_session_fixture_with_initial_heartbeat`.
- Added `live_pending_session_fixture` for pending-session WebSocket connection setup.

## Tests Executed

- `cargo test -p burd-control-plane` passed: 100 passed, 18 ignored.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane live_control_channel_websocket_flow_acknowledges_duplicate_and_revocation -- --ignored` passed: 1 passed.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored` passed: 18 passed.
- `cargo fmt --all --check` passed.
- `cargo test --workspace` passed.
- `cargo build --workspace` passed.

## Tests Not Executed Yet In This Pass

- Agent daemon end-to-end remote session loop.
- Real GPU telemetry streaming through the WebSocket channel.
- Real CUDA/VRAM/GEMM/LLM Proof of Capability execution.
- Scheduler, marketplace, billing, Pix, payout, and secure runtime end-to-end flows.

## Remaining Risks

- The live helper still cleans up the temporary schema only on successful completion; a panic can leave disposable schemas in the isolated test database.
- WebSocket telemetry behavior is still covered by HTTP ingestion and lower-level validation, not a live socket telemetry stream.
- The agent-side reconnect/backoff loop is not covered here because no agent daemon exists yet.

## Deferred Items

- Add live WebSocket telemetry batch coverage once the test fixture can produce a signed telemetry window cheaply.
- Add agent-daemon remote session E2E coverage when the daemon exists.
- Move shared live HTTP/WebSocket fixtures into a dedicated integration support module if `http.rs` grows further.

## Recommended Next Hardening PRs

- Add live telemetry-to-proof linkage coverage.
- Add agent-side remote session retry/backoff contract coverage when daemon work starts.
- Split live fixtures out of `http.rs` if another hardening pass needs them.