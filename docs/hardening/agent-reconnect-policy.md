# Agent Reconnect Policy

## Summary

This hardening pass makes the existing `burd-agent remote-session connect`
retry behavior bounded, classified, observable, and testable. Repeated
WebSocket failures now continue increasing the backoff even when the HTTP
session resume request succeeds. The delay resets only after the backend has
acknowledged a heartbeat on the connection.

No daemon, service installer, background scheduler, remote proof runner, job
runtime, marketplace feature, billing automation, Pix gateway, or payout flow
was added.

## Scope Audited

- `crates/burd-agent/src/remote_enrollment.rs`
- `crates/burd-agent/src/remote_session.rs`
- Device credential refresh
- Session start and resume
- WebSocket handshake classification
- Reconnect scheduling and structured stderr events
- BN-03 and current-state documentation

## Implemented For Real

- Control-plane HTTP failures preserve transport, status, machine-readable error
  code, and contract failure categories inside the Agent.
- Transient transport, HTTP `408`/`429`/`5xx`, and conflict failures retry.
- Revoked or invalid credentials stop the command instead of retrying forever.
- Missing or expired WebSocket sessions clear local resume state and create a
  new remote session.
- Retry ceilings grow exponentially from one second to
  `--max-reconnect-delay-seconds`.
- Delays use deterministic per-agent jitter between half of the current ceiling
  and the ceiling.
- The retry counter resets only after an accepted heartbeat proves the
  connection is usable.
- `remote_session_retry_scheduled` logs attempt, delay, ceiling, failure kind,
  action, and whether the previous connection was stable.

## Still Planned, Mock, Or Local

- The retry counter is in-memory and resets when the command process restarts.
- `remote-session connect` remains a foreground command, not a supervised Agent
  daemon or operating-system service.
- There is no durable daemon health state, process supervisor integration, or
  automatic startup policy.

## Bugs Found

- A successful HTTP `start_or_resume` reset the delay to one second even when
  every following WebSocket connection failed before becoming usable.
- Credential refresh failures before a connection escaped the loop immediately,
  so transient backend or network failures did not use reconnect policy.
- Session expiration and credential revocation were partly distinguished by
  matching human-readable error strings.
- WebSocket handshake status was flattened into an unclassified string.
- Synchronous credential/session preparation ran directly inside the async retry
  loop.
- The only Agent remote-session unit test covered credential refresh timing.

## Bugs Fixed

- All retryable preparation and connection failures now share one bounded
  policy.
- HTTP status and backend error code drive failure classification without
  parsing display text.
- WebSocket `404`/`410` restarts session state; `400`/`401`/`403` stops; other
  transport and server failures retry.
- Stable connections reset retry progression only after heartbeat
  acknowledgement.
- Blocking enrollment/session preparation runs through `spawn_blocking`.

## Architecture And Overengineering

- Two small abstractions were retained because they protect real boundaries:
  typed control-plane request errors and the reconnect policy.
- No event bus, listener, service framework, persistent worker, or new crate was
  introduced.
- CLI arguments and persisted enrollment/session JSON contracts are unchanged.

## Events And Listeners

- No event bus, domain listener, outbox, or background dispatcher changed.
- Existing stderr JSON logging now emits one consolidated
  `remote_session_retry_scheduled` event per scheduled retry.

## Migrations And Database

- No migration or database behavior changed.
- PostgreSQL tests are still run as regression coverage before publication, but
  this PR does not add a database path.

## Security Findings

- Revoked and unauthorized credentials fail terminally instead of creating an
  endless reconnect loop.
- Request classification stores only status, backend error code, and the
  backend's redacted message; response bodies and authorization headers are not
  logged.
- The jitter seed is derived from the local machine ID but the machine ID and
  seed are never emitted.
- Private keys, device credentials, resume tokens, and authorization headers are
  not added to retry events.

## Performance Findings

- Hardware registration, file access, credential refresh, and session resume are
  moved off the async executor through `spawn_blocking`.
- Backoff remains bounded by the existing CLI option and uses no busy loop.
- No dependency was added for jitter or retry scheduling.

## Tests Added

- Typed control-plane rejection preserves status and error code.
- Backoff progression is deterministic, exponentially bounded, jittered, and
  resettable after a stable connection.
- Backend `5xx` remains retryable while revoked and unauthorized failures stop.
- WebSocket `410` restarts session state, `403` stops, and `503` retries.
- HTTP `404`/`410` can restart persisted sessions even without a valid JSON
  error envelope.

## Tests Executed

- `cargo fmt --all --check` passed.
- `cargo test -p burd-agent -p burd-protocol` passed: 6 Agent tests and
  31 protocol tests.
- `cargo test -p burd-control-plane -- --ignored` passed against the isolated
  PostgreSQL database: 19 passed.
- `cargo test --workspace` passed. The expected slow real-hardware test remained
  ignored, and PostgreSQL tests were executed separately above.
- `cargo build --workspace` passed.
- `cargo clippy --workspace --all-targets` passed. It reported only pre-existing
  warnings outside the changed files in `third_party/llmfit`, `burd-bench`, and
  `burd-control-plane`.

## Commands That Failed

- No code validation command failed.
- The first sandboxed `docker start burd-postgres-test` call was denied access
  to Docker Desktop. The approved retry succeeded, and all PostgreSQL tests
  passed.

## Remaining Risks

- A deterministic jitter sequence decorrelates different machines but repeats
  after a process restart on the same machine.
- The command has no durable retry-attempt history or supervisor-level crash
  recovery.
- Mid-connection protocol errors other than explicit revocation remain
  retryable; repeated failures are bounded but still require operator diagnosis.

## Recommended Next Hardening PR

Add a live Agent-to-Control-Plane integration harness that runs the real Agent
connection loop against an isolated backend, injects socket loss and session
expiry, and verifies retry/terminal behavior without relying only on pure policy
tests.
