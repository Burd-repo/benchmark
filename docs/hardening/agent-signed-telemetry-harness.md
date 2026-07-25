# Live Agent Signed Telemetry Integration Harness

## Summary

This hardening pass extends the live Agent-to-Control-Plane harness with a
deterministic, test-only NVIDIA telemetry source. The test runs the real Agent
connection loop, canonical batch builder, Ed25519 signer, WebSocket transport,
Control Plane verifier, and PostgreSQL transaction without requiring NVIDIA
hardware on the test runner.

The production `nvidia-smi` collector, CLI surface, telemetry protocol, backend
routes, and database schema are unchanged. No remote proof runner, benchmark
runner, daemon, scheduler, job runtime, marketplace workflow, Pix gateway, or
payout execution was added.

## Scope Audited

- Agent telemetry collection boundary
- Canonical telemetry hashing and Ed25519 signing
- WebSocket control and sample sequences
- Telemetry acknowledgement persistence
- Session resume with telemetry sequence continuation
- Backend validation, normalized sample persistence, and rejection handling
- BN-03 and BN-04 current-state documentation

## Implemented For Real

- Production Agent telemetry still uses `collect_nvidia_telemetry`.
- The Agent still signs batches with the enrolled device key and sends them over
  the authenticated outbound control channel.
- The Control Plane still verifies identity binding, physical ranges, canonical
  hash, active-key signature, sequence, timing policy, and persistence inside
  the existing transaction.
- A rejected telemetry batch remains nonfatal to the remote heartbeat session.

## Test-Only Support

- `integration-test-support` exposes a cancelable Agent entry point that accepts
  a telemetry collector function pointer.
- The default integration entry point and production CLI both select the real
  NVIDIA collector explicitly.
- The deterministic source can emit valid samples, simulate an unavailable
  collector, or emit an out-of-range utilization value for backend rejection.
- The source, mode controls, counters, short timing, TCP proxy, and assertions
  exist only in the integration test target.

## Bugs Found And Fixed

- No production telemetry defect was found by this pass.
- The previous harness could not prove that Agent-produced telemetry was signed,
  acknowledged, persisted, resumed, or rejected correctly. The new scenario
  closes that verification gap without weakening production validation.

## Architecture And Overengineering

- No collector trait hierarchy, dependency-injection framework, event bus, mock
  server, or alternate production transport was introduced.
- One function pointer protects the hardware collection boundary needed by the
  integration test.
- `burd-hardware` is an explicit Control Plane development dependency only so
  the external integration test can construct the existing collection type.

## Events And Listeners

- No event bus, listener, outbox, or background dispatcher changed.
- Existing structured `gpu_telemetry_unavailable`,
  `gpu_telemetry_rejected`, retry, and HTTP events are exercised as-is.
- Existing accepted telemetry audit persistence remains owned by the backend
  transaction.

## Migrations And Database

- No migration or production schema changed.
- The test uses a UUID-scoped PostgreSQL schema and the normal migration path.
- Assertions cover the authoritative `telemetry_batches`,
  `gpu_telemetry_samples`, `provider_sessions`, `provider_public_keys`, and
  `audit_events` data paths.
- Rejected telemetry is verified absent from persisted batch history.

## Security Findings

- The test recalculates the canonical hash and verifies the stored Ed25519
  signature against the enrolled backend public key.
- Private keys, device credentials, enrollment tokens, resume tokens, and
  authorization headers are neither logged nor asserted.
- The invalid fixture is rejected by the production physical-range validator;
  no test bypass is added to backend verification.

## Performance Findings

- The deterministic source runs through the existing `spawn_blocking` boundary.
- Production collection cost and cadence are unchanged.
- Telemetry rejection disables further collection for that live connection,
  while heartbeat processing continues without a retry loop or CPU spin.

## Tests Added

The live PostgreSQL test verifies:

- Agent-produced deterministic NVIDIA sample collection;
- canonical payload hash and Ed25519 signature;
- backend verification metadata and normalized sample count;
- matching control and sample sequence domains;
- local telemetry ACK sequence persistence;
- socket loss and resume without duplicate sample sequences;
- backend rejection of utilization above 100 percent;
- no persistence of the rejected batch;
- telemetry stream disablement after rejection;
- continued remote heartbeats after telemetry rejection.

## Tests Executed

- `cargo test -p burd-control-plane --test agent_remote_session live_agent_signed_telemetry_persists_ack_resumes_and_handles_rejection -- --ignored`
  passed against isolated PostgreSQL.
- `cargo test -p burd-agent -p burd-control-plane` passed: 8 Agent tests and
  100 non-PostgreSQL Control Plane tests; 21 database/harness tests remained
  ignored in the default command.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored`
  passed: 19 existing PostgreSQL tests and both live Agent harnesses.
- `cargo test --workspace` passed: 222 tests; the real-hardware test and the 21
  PostgreSQL/harness tests were executed separately or remained ignored as
  appropriate.
- `cargo build --workspace` passed.
- `cargo clippy --workspace --all-targets` passed. It reported only pre-existing
  warnings outside the changed code.
- `cargo fmt --all --check` passed.

## Commands That Failed During Development

- `apply_patch` could not initialize the Windows restricted-token sandbox. The
  same scoped edits were applied with occurrence-checked UTF-8 writes.
- The first compile of the integration target found an `i64`/`u64` comparison
  mismatch in a test assertion. The comparison now uses an explicit checked
  conversion.

## Tests Not Executed

- The pre-existing slow real-hardware integration test was not executed. It
  probes the developer machine and is unrelated to the deterministic source.
- Real NVIDIA telemetry was not collected in CI-style validation; the harness
  intentionally proves protocol behavior without presenting fixture values as
  physical measurements.

## Remaining Risks

- The deterministic source proves Agent behavior but is not evidence that a CI
  runner collected real NVIDIA telemetry.
- NVIDIA driver and `nvidia-smi` compatibility still depend on real-host and
  slow hardware tests.
- A telemetry rejection disables collection only for the current connection;
  reconnecting starts a fresh telemetry attempt and may repeat a persistent
  policy error.
- At the time of this harness, the Agent did not execute backend-issued Proof of
  Capability workloads or bind telemetry capture to execution. The subsequent
  BN-06 Agent runner closes that gap.
- The foreground Agent command still lacks service supervision and durable
  retry history.

## Recommended Next PR (Completed)

The subsequent BN-06 Agent runner implemented this recommendation:
pick up backend-issued challenges, enforce nonce/fingerprint/GPU/artifact and
expiry bindings, execute only the already-defined approved proof profiles,
capture the required telemetry window, sign the independent response, and
submit it through the existing Control Plane contract. Keep mock/local
capability results clearly separate from remotely verified proof.