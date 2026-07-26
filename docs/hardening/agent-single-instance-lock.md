# Agent Remote Session Single-Instance Lock

Date: 2026-07-25

## Summary

This hardening pass prevents two `remote-session connect` processes from using
the same canonical Agent state directory at the same time. It uses the Rust
standard library's cross-platform exclusive file lock and adds no dependency,
backend contract, daemon, installer, or service manager.

## Problem

The remote session, telemetry, enrollment credential refresh, and BN-06 proof
worker share persisted sequence and attempt state. Two foreground Agent
processes using the same state directory could race on:

- `remote-session.json` control and telemetry sequences;
- credential refresh state;
- `remote-proof-attempts.json`;
- duplicate WebSocket connections and challenge execution.

The backend already rejects duplicate control channels, but that happens after
local processes can read and mutate shared files. The Agent needs a local
process boundary before starting the runtime.

## Implemented Behavior

- Every production and integration `remote-session connect` entrypoint acquires
  an exclusive `remote-session.lock` in the canonical Agent state directory.
- A second process fails immediately with a clear local error before identity,
  network, WebSocket, telemetry, or proof work starts.
- The lock is held by the open file handle for the complete connection,
  reconnect, shutdown, and final status-read lifecycle.
- The operating system releases the lock when the handle closes, including
  normal shutdown, error unwinding, or process termination.
- The lock file intentionally remains after release. File presence is not used
  as proof that another process is active, so there is no stale-PID cleanup
  algorithm.
- A later process opens the same file and acquires the OS lock normally.
- The file contains only schema version, process ID, and acquisition timestamp.
  It contains no credential, token, signature, key, provider payload, or
  backend response.
- Existing symbolic-link and non-file lock paths are rejected before opening.
- `remote-session status` and diagnostic commands do not acquire this lock and
  remain usable while the connection is running.

## Failure Behavior

- Lock contention returns a local error and does not retry.
- I/O and unsupported-filesystem locking errors remain distinct from
  contention and include the lock path, but no secret.
- Metadata write/sync failure aborts startup while the temporary handle still
  owns the lock; dropping that handle releases it.
- The lock does not infer liveness from the recorded PID.

## Compatibility

No CLI flag, output JSON, HTTP route, OpenAPI schema, WebSocket message,
PostgreSQL migration, signed payload, or Control Plane behavior changed.

The lock protects one remote-session process per canonical local state
directory. It is not backend session authority, device revocation, remote
attestation, or operating-system service supervision.

## Tests Added

- a second independent file handle cannot acquire the same lock;
- a real child process holds the lock, the parent observes contention, and
  forced process termination releases it;
- dropping the first guard allows reacquisition;
- the persistent lock file does not create a false stale-lock failure;
- previous file contents are overwritten only after exclusive acquisition;
- metadata remains bounded and contains no sensitive field names;
- an existing non-file lock path is rejected.

The existing ignored PostgreSQL Agent/WebSocket harnesses exercise the same
locked integration entrypoints.

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
```

Results:

- `cargo build --workspace`: passed;
- `cargo test --workspace`: 245 passed and 25 environment-dependent tests
  ignored;
- `cargo test -p burd-agent`: 25 passed and the live Ollama test ignored;
- `cargo clippy -p burd-agent --all-targets`: passed with existing warnings in
  `third_party/llmfit` and `burd-bench`; no warning was introduced by this pass;
- PostgreSQL ignored suite: 20 Control Plane tests and 3 Agent/WebSocket
  integration tests passed.

The live Ollama test was not run because it requires a local Ollama service
with an installed model. The PostgreSQL suite used the isolated
`burd-postgres-test` Docker database.

## Remaining Limitations

- The Agent remains a foreground command. This does not add Windows Service,
  systemd, installer, autostart, health management, or update policy.
- The lock coordinates remote-session processes only. Concurrent identity,
  enrollment, or other state-mutating CLI commands are not yet coordinated with
  a running session.
- File-lock guarantees depend on a local filesystem that implements the Rust
  standard library lock contract. Network/shared state directories are not a
  supported deployment model.
- A user with write access to the state directory can still tamper with local
  files; this is not a privilege or sandbox boundary.
- Physical CUDA compatibility and hardware-backed attestation remain separate
  work.

## Recommended Next Work

1. Freeze which state-mutating maintenance commands may run while the future
   service is active, then either share this lock or add explicit maintenance
   coordination.
2. Define service startup, graceful shutdown, recovery, credential refresh, and
   update policy before adding Windows Service or systemd packaging.
3. Run the existing BN-06 physical compatibility matrix on controlled NVIDIA
   hosts independently of service packaging.
