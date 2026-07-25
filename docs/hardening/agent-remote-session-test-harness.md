# Live Agent Remote Session Integration Harness

## Summary

This hardening pass runs the real Burd Agent enrollment and remote-session loop
against the real Axum control plane and an isolated PostgreSQL schema. A
controllable TCP proxy injects socket loss and backend unavailability without
adding a production transport abstraction.

The harness exposed a real heartbeat-path performance bug: the Agent repeated
full hardware registration on every heartbeat and telemetry batch. The Agent
now collects registration during blocking connection preparation and reuses the
enrollment-bound fingerprint for the lifetime of that connection.

No daemon, operating-system service, scheduler, job runtime, remote proof
runner, marketplace feature, billing automation, Pix gateway, or payout flow
was added.

## Scope Audited

- Agent enrollment and persisted remote identity
- Agent session start/resume and WebSocket control loop
- Heartbeat acknowledgement and sequence persistence
- Reconnect policy under socket loss and backend unavailability
- Server-side session expiry and replacement
- Administrative session revocation
- Control Plane Axum router and PostgreSQL persistence
- BN-03 and current-state documentation

## Implemented For Real

- `burd-agent` now has a library target used by its existing binary target; the
  CLI commands and output contracts are unchanged.
- The production foreground command translates `Ctrl+C` into the same internal
  shutdown signal used by the integration harness.
- Hardware registration runs once per connection attempt inside
  `spawn_blocking`.
- Heartbeats and signed telemetry reuse that attempt's fingerprint instead of
  invoking hardware detection inside the periodic async path.
- The integration test performs real HTTP enrollment, Ed25519 proof, session
  start, WebSocket heartbeat, resume, expiry replacement, and revocation.
- The integration test verifies PostgreSQL session state, monotonic sequence,
  and `provider_session.resumed` audit events.

## Test-Only Support

- `integration-test-support` exposes the Agent's cancelable async connection
  entry point only when explicitly enabled.
- The TCP fault proxy and short session timing exist only in
  `crates/burd-control-plane/tests/agent_remote_session.rs`.
- The test uses a unique PostgreSQL schema and a unique Agent state directory,
  then removes both after success.
- Telemetry collection is disabled in this harness because it must not depend on
  NVIDIA hardware being present on the CI runner.

## Bugs Found

- Full hardware/provider registration was rebuilt synchronously after every
  heartbeat interval.
- The same expensive registration path was repeated when preparing each signed
  telemetry batch.
- Short heartbeat policies could close an otherwise valid channel before the
  Agent sent its first heartbeat.
- Policy-only tests could not prove that HTTP resume, WebSocket reconnect,
  server TTL, local session replacement, and revocation worked together.

## Bugs Fixed

- Hardware detection was removed from the heartbeat loop.
- Telemetry signing now receives the connection's enrollment-bound fingerprint
  explicitly.
- Blocking registration remains outside the async executor.
- Shutdown can interrupt active connections and retry sleeps through one
  explicit signal path.

## Architecture And Overengineering

- No generic transport trait, event bus, listener framework, daemon shell, or
  reusable fault-injection subsystem was introduced.
- The Agent library target removes duplicate binary-only compilation boundaries
  and lets tests call production code directly.
- The async entry point is feature-gated to avoid presenting test lifecycle
  control as a new supported product API.
- The raw TCP proxy is local to one integration test and does not change
  production Control Plane state or routing.

## Events And Listeners

- No event bus, domain listener, outbox, or background dispatcher changed.
- Existing retry and HTTP structured events are exercised without adding a new
  event contract.
- The test verifies the existing `provider_session.resumed` audit event instead
  of inferring resume only from status fields.

## Migrations And Database

- No migration or production schema changed.
- The harness uses the existing `Database::migrate` path and a UUID-scoped
  PostgreSQL schema.
- Existing session rows, heartbeats, and audit events remain authoritative.
- All 19 existing ignored PostgreSQL tests and the new Agent harness passed.

## Security Findings

- Device credentials, enrollment tokens, private keys, and resume tokens are not
  printed or asserted by the test.
- Temporary Agent identity state is isolated under `target` and removed after
  the scenario.
- Administrative revocation is sent through the real authenticated HTTP route
  and terminates the Agent loop.
- No authorization header, credential, or private key was added to retry logs.

## Performance Findings

- Repeated hardware detection and registration serialization were removed from
  heartbeat and telemetry send paths.
- One registration snapshot is reused only for a connection attempt; reconnect
  still recollects state so changed hardware is not silently carried forever.
- Retry sleeps remain bounded and do not busy-loop while the backend is down.

## Tests Added

- Telemetry batch-size validation accepts `1..=64` and rejects boundary values
  outside that range.
- The shutdown receiver wakes control-loop waiters.
- The live Agent harness verifies:
  - first acknowledged heartbeat and persisted sequence;
  - socket loss followed by resume of the same session;
  - persisted resume audit event;
  - backend unavailability longer than session TTL;
  - server-side expiry and creation of a replacement session;
  - administrative revocation terminating the Agent loop.

## Tests Executed

- `cargo test -p burd-agent -p burd-control-plane` passed: 8 Agent tests and 100
  non-PostgreSQL Control Plane tests passed; 20 database tests remained ignored
  in this default command.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored`
  passed: 19 existing PostgreSQL tests plus the new live Agent harness.
- `cargo test --workspace` passed. The existing slow real-hardware test remained
  ignored; all PostgreSQL tests were executed separately above.
- `cargo build --workspace` passed.
- `cargo clippy --workspace --all-targets` passed after replacing the harness's
  process-wide standard mutex with an async-aware mutex. Remaining warnings are
  pre-existing outside this change.
- `cargo fmt --all --check` passed.

## Commands That Failed During Development

- The first focused harness run failed because repeated hardware registration
  prevented the first heartbeat before the test server timeout. That failure
  exposed the production-path bug fixed here.
- The second focused run used an unrealistically short three-second session TTL;
  it correctly expired during connection preparation instead of exercising the
  intended short-outage resume case. The fixture now separates short outage
  from deliberate expiry with an eight-second TTL.
- `apply_patch` could not initialize the Windows restricted-token sandbox. The
  same scoped edits were applied with occurrence-checked UTF-8 writes.

## Tests Not Executed

- The pre-existing slow real-hardware integration test was not executed. It
  probes the developer machine and is unrelated to the deterministic remote
  session scenario.
- Real NVIDIA telemetry was not collected by this harness because CI and local
  test machines cannot be assumed to expose compatible GPU hardware.

## Remaining Risks

- `remote-session connect` remains a foreground command without service
  supervision or durable retry history.
- Hardware registration is recollected on every reconnect attempt; it is off the
  async executor but can lengthen recovery on slow hosts.
- The harness verifies Agent heartbeat and lifecycle behavior, but not
  Agent-produced signed telemetry through the same connection.
- The TCP proxy models connection loss and refusal, not packet loss, latency, or
  half-open network behavior.

## Recommended Next Hardening PR

Add a deterministic, test-only NVIDIA telemetry source to the live Agent
harness so Agent-produced batch hashing, Ed25519 signing, control/sample
sequences, acknowledgement persistence, and rejection handling can be verified
without requiring GPU hardware on CI. Keep the production collector unchanged.