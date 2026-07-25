# Live WebSocket Telemetry Streaming

## Summary

This hardening pass adds live PostgreSQL integration coverage for the normal
BN-04 telemetry path: an enrolled device sends a signed telemetry batch through
the authenticated BN-03 WebSocket control channel, receives `telemetry_ack`, and
the backend persists the batch and normalized GPU samples transactionally.

It also fixes stale `sequence_ack` values in backend-initiated
`session_revoked` messages after the connection has accepted heartbeat or
telemetry messages.

No agent daemon, NVIDIA collector change, proof workload runner, scheduler,
job runtime, marketplace feature, Pix gateway, billing automation, or payout
flow was added.

## Scope Audited

- `crates/burd-control-plane/src/http.rs`
- Authenticated WebSocket control-channel handling
- BN-04 signed telemetry ingestion and receipts
- PostgreSQL telemetry batch and normalized sample persistence
- Control-message replay rejection
- Backend-initiated session revocation acknowledgements

## Implemented For Real

- Signed `telemetry_batch` messages are accepted through the live WebSocket
  control channel.
- Successful ingestion returns `telemetry_ack` with the accepted control and
  sample sequences.
- Accepted batches and normalized GPU samples are stored in PostgreSQL.
- Replayed control sequences return `telemetry_rejected` without duplicating
  telemetry rows.
- `session_revoked.sequence_ack` now reports the latest backend-persisted
  control sequence.

## Still Planned, Mock, Or Local

- The telemetry payload in this integration test is deterministic fixture data,
  not a live `nvidia-smi`, NVML, or DCGM collection.
- This pass does not add a continuously running agent daemon.
- Proof telemetry capture remains manually linked through accepted batch hashes;
  this pass does not orchestrate workload-window capture in the agent.

## Bugs Found

- Backend-initiated `session_revoked` messages used the sequence captured when
  the WebSocket was authorized. After later heartbeat or telemetry messages,
  the acknowledgement could therefore be stale.
- The WebSocket telemetry branch had no live database integration test; only
  the authenticated HTTP fallback was exercised end to end.

## Bugs Fixed

- Revocation delivery now reloads the authoritative remote-session sequence
  before constructing `session_revoked`, with the connection-start sequence
  retained only as a database-error fallback.
- The live control-channel test now verifies signed WebSocket telemetry
  acceptance, receipt fields, latest-telemetry retrieval, normalized database
  rows, replay rejection, and revocation sequence continuity.

## Overengineering Removed

- No production abstraction or event layer was added.
- The test reuses the existing enrollment, session, signing, telemetry, and
  live-server fixtures.

## Events And Listeners

- No event bus, listener, background dispatch, or outbox behavior changed.

## Migrations And Database

- No migration was added or edited.
- The test reads existing `telemetry_batches` and `gpu_telemetry_samples` rows
  to verify transactional persistence and absence of replay duplication.

## Security Findings

- The live path exercises enrolled-key Ed25519 verification over the
  authenticated WebSocket connection.
- A replayed control sequence is rejected and does not create a second batch.
- No private key, device credential, resume token, authorization header, or
  database URL is logged or committed.

## Performance Findings

- The revocation path adds one bounded primary-key session lookup only when the
  backend is already closing a revoked channel.
- Normal heartbeat and telemetry request paths are unchanged.

## Tests Added Or Changed

- `http::tests::live_control_channel_websocket_flow_persists_telemetry_and_rejects_replay`
  now covers WebSocket telemetry acceptance, acknowledgement, persistence,
  replay rejection, and current-sequence revocation.

## Tests Executed

- `cargo fmt --all --check` passed.
- `cargo test -p burd-control-plane` passed: 100 passed, 19 ignored.
- The focused ignored WebSocket telemetry test passed against the isolated
  PostgreSQL test database: 1 passed.
- `cargo test -p burd-control-plane -- --ignored` passed against PostgreSQL:
  19 passed.
- `cargo test --workspace` passed. The expected real-hardware test remained
  ignored, and the PostgreSQL suite was executed separately above.
- `cargo build --workspace` passed.

The workspace test and build emitted two pre-existing dead-code warnings from
`third_party/llmfit`; no third-party code was changed.

## Commands That Failed

- No validation command failed.

## Remaining Risks

- A valid signed telemetry batch proves which enrolled key reported the data;
  it does not provide hardware-backed attestation.
- The agent connection command remains user-started rather than a supervised
  daemon with durable retry state.
- The live fixture cleans its schema only after successful completion; a panic
  can leave a disposable test schema for later cleanup.

## Recommended Next Hardening PR

Add explicit agent-side remote-session reconnect state with bounded exponential
backoff, jitter, retry observability, and tests that distinguish transient
transport failure from revoked or expired credentials.
