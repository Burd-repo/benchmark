# Agent State Maintenance Lock

Date: 2026-07-25

## Summary

This hardening pass extends the Agent single-instance lock to critical local
state maintenance. A running `remote-session connect` process and commands that
can replace identity, enrollment, credential, or API-token state now contend on
the same operating-system file lock.

The physical filename remains `remote-session.lock`. Keeping that name is
intentional: Agents from this pass still contend with the immediately preceding
version instead of silently using a different lock file.

## Protected Operations

The CLI acquires the state lock before these commands perform validation,
network access, secret loading, or local writes:

- `identity init`;
- `identity migrate`;
- `identity rotate-key`;
- `enrollment enroll`;
- `enrollment refresh-credential`;
- `api-token create`;
- `api-token rotate`.

`remote-session connect` continues to acquire the same lock inside the session
runtime. Its lock covers identity loading, connection and reconnection,
credential refresh, telemetry, proof execution, shutdown, and final status
loading.

This prevents:

- replacing the Ed25519 key while telemetry or proof responses are being
  signed;
- enrolling a different remote device while a session uses the persisted
  enrollment;
- racing a manual credential refresh with the session refresh path;
- rewriting `agent.json` while the proof worker reloads identity and key state;
- starting maintenance between session validation and lock acquisition.

## Operations Left Available

These commands do not acquire the maintenance lock:

- `identity show`;
- `enrollment status`;
- `remote-session status`;
- `api-token show`;
- hardware, readiness, score, trust, benchmark, report, and other diagnostic
  commands.

The remote session acquires its own lock and therefore is not classified as a
maintenance command by the CLI dispatcher.

Independent benchmark, report, history, uptime, action, and local-session files
are not brought under this lock. This pass protects the identity and remote
session trust boundary; it is not a global one-process policy for every Agent
command.

## Lock Contract

- The lock remains an exclusive `std::fs::File` lock in the canonical Agent
  state directory.
- The open file handle is the authority. File presence and recorded PID do not
  determine liveness.
- Metadata schema `2` stores only operation identifier, PID, and acquisition
  timestamp.
- Operation identifiers are a closed Rust enum and cannot contain command-line
  input, tokens, URLs, provider payloads, or backend responses.
- The operating system releases the lock after normal return, error unwinding,
  or process termination.
- Symbolic-link and non-file paths continue to fail closed.

## Failure Behavior

Contention fails immediately. The error identifies the requested operation and
state directory, but does not claim which process owns the lock because lock
metadata is diagnostic and may be stale after release.

The command does not retry automatically. Operators must wait for another
maintenance command to finish or stop the foreground remote session before
changing protected state.

## Compatibility

No command, flag, successful JSON response, HTTP route, OpenAPI schema,
WebSocket message, signed payload, PostgreSQL schema, or backend behavior
changed.

The observable change is intentional: protected maintenance commands now
return an error instead of running concurrently with a remote session or
another protected command.

## Tests Added Or Updated

- every protected CLI command maps to its exact lock operation;
- status, diagnostic, and self-locking remote-session commands remain outside
  CLI maintenance locking;
- one operation excludes a different operation using an independent handle;
- lock metadata records the bounded operation identifier without secret
  values;
- process termination releases the lock for a different maintenance
  operation;
- non-file lock paths continue to fail closed.

The PostgreSQL Agent/WebSocket integration tests use the same remote-session
entrypoints and validate that the generalized guard does not change protocol or
backend behavior.

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
- `cargo test --workspace`: 247 passed and 25 environment-dependent tests
  ignored;
- `cargo test -p burd-agent`: 27 passed and the live Ollama test ignored;
- `cargo clippy -p burd-agent --all-targets`: passed with existing warnings in
  `third_party/llmfit` and `burd-bench`; this pass introduced no warning;
- PostgreSQL ignored suite: 20 Control Plane tests and 3 Agent/WebSocket
  integration tests passed.

The live Ollama test was not run because it requires a local Ollama service
with an installed model. PostgreSQL validation used the isolated
`burd-postgres-test` Docker database.

## Remaining Limitations

- The Agent is still a foreground command without Windows Service, systemd,
  installer, autostart, restart, or update policy.
- Read-only status commands do not take a shared lock. Canonical local state
  writers now use per-file atomic replacement, documented in
  `docs/hardening/atomic-local-state-persistence.md`.
- Independent local histories and action logs still have no general
  multi-process write coordination.
- File-lock guarantees require a local filesystem implementing the Rust
  standard-library lock contract.
- This is not a privilege boundary, backend authority, remote attestation, or
  hardware-backed key store.
- Physical CUDA/Ollama compatibility across supported NVIDIA hosts remains
  separate validation work.

## Recommended Next Work

1. Freeze service startup, graceful shutdown, recovery, credential refresh, and
   update policy before adding Windows Service or systemd packaging.
2. Run the BN-06 physical compatibility matrix on controlled NVIDIA Windows and
   Linux hosts.
