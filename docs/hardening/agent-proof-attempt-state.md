# Agent Proof Attempt State And Foreground Supervision

Date: 2026-07-25

## Summary

This hardening pass makes the existing BN-06 foreground proof worker durable
across Agent restarts and supervised by the foreground remote-session process.
It does not add an operating-system service, background daemon, new proof
profile, backend endpoint, or signed protocol field.

## Problem

The worker previously kept one failed attempt only in memory. If the Agent
restarted before the challenge expired, it could fetch and execute the same
expensive challenge again. A locally rejected challenge with an invalid expiry
also received no effective cooldown and could cause a tight polling loop.

The control loop spawned the proof worker but observed its result only after the
control loop ended. A proof worker that failed early could therefore leave an
apparently connected foreground session running without proof processing.

## Implemented Behavior

- `remote-proof-attempts.json` is stored under the canonical Agent state
  directory.
- The local schema stores only challenge ID, session ID, outcome, record time,
  and suppression deadline.
- Nonces, credentials, resume tokens, signatures, payloads, private keys, model
  prompts, telemetry, and free-form errors are not persisted.
- `rejected_locally` and `attempt_failed` records suppress another attempt for
  the same session until the server challenge expires.
- `submitted` records remain diagnostic history and do not suppress future
  challenges.
- Invalid or already-expired challenge timestamps receive one polling interval
  of cooldown instead of immediately refetching in a tight loop.
- The history deduplicates by challenge and session and retains at most 64
  records.
- The state file is capped at 256 KiB, schema-validated on startup, written to a
  synced temporary file, and atomically replaced.
- Missing state starts with an empty history. Malformed, oversized, or unknown
  schema state fails the proof worker closed instead of silently discarding
  retry protection.
- File reads and writes run through `spawn_blocking`; the async worker does not
  perform filesystem I/O directly.
- The foreground supervisor now observes the control loop, proof worker, and
  external shutdown signal together. Unexpected proof worker exit shuts down
  the control loop and returns an error.

## Compatibility

No HTTP route, OpenAPI schema, PostgreSQL migration, Control Plane behavior,
challenge schema, response hash, signature message, CLI flag, or WebSocket
message changed. Existing `remote-session connect --proofs` behavior remains
foreground-only.

This state is local operational metadata. It is not signed evidence, remote
verification, a backend retry budget, trust input, marketplace approval, or
Proof of Capability result.

## Tests Added

- failed attempt survives store reload and suppresses only the same session;
- submitted and expired records do not suppress;
- duplicate records are replaced and history remains limited to 64;
- serialized state contains none of the prohibited sensitive field names;
- malformed schema and oversized files fail closed;
- invalid challenge expiry receives a bounded polling cooldown;
- foreground supervisor propagates proof worker failures.

## Validation

Run during this pass:

```powershell
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
cargo test -p burd-agent
cargo clippy -p burd-agent --all-targets
$env:BURD_CONTROL_TEST_DATABASE_URL='postgres://burd:burd@127.0.0.1:5432/burd_test'
cargo test -p burd-control-plane -- --ignored
cargo test -p burd-control-plane live_agent_executes_and_submits_a_verified_remote_proof -- --ignored
```

All commands passed. The workspace run completed 241 normal tests with 25
conditional tests ignored. The PostgreSQL run completed all 20 database tests
and three Agent/WebSocket integration harnesses. The focused BN-06 harness was
then repeated after adding assertions for persisted outcome, challenge/session
binding, and secret redaction.

Clippy reported only existing warnings in `burd-bench` and
`third_party/llmfit`; none originated from this pass.

## Remaining Limitations

- The Agent is still a foreground command. There is no Windows Service,
  systemd unit, installer, autostart, OS health manager, or release updater.
- At the time of this pass, the state had no cross-process lock. The later
  `agent-single-instance-lock.md` pass prevents duplicate remote-session
  processes for the same canonical state directory.
- A failed attempt stores a coarse outcome, not a free-form local error. This is
  intentional to keep the file bounded and avoid persisting sensitive backend
  or runtime messages.
- The Agent does not persist a signed response for independent resubmission
  after a transport failure. The backend challenge remains authoritative.
- CUDA UUID binding, VRAM residency, cuBLAS, physical contention, and the full
  NVIDIA compatibility matrix still require controlled NVIDIA hosts.
- This is software-level retry protection, not hardware-backed attestation.

## Recommended Next Work

1. Run the existing production executor on controlled NVIDIA Windows and Linux
   hosts and record the CUDA compatibility matrix.
2. Completed by `agent-single-instance-lock.md`: add an explicit remote-session
   single-instance lock before service packaging.
3. Add OS service packaging only after startup, shutdown, credential refresh,
   state corruption, and update policies are frozen.
