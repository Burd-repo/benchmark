# Agent Lifecycle Readiness And Cooperative Startup Shutdown

Date: 2026-07-27

## Summary

This hardening pass adds a local, machine-readable lifecycle contract to the
existing foreground `burd-agent remote-session connect` process. It also makes
connection preparation and active credential refresh observe cooperative
shutdown, and extends the real PostgreSQL/WebSocket harness across stop,
reconnect, Control Plane outage, recovery, and revocation.

This work does not add an operating-system service, daemon, autostart, automatic
update, job runtime, scheduler, or new remote protocol.

## Local Contract

`burd-agent remote-session lifecycle --json` returns:

- `schema_version`: `burd.agent.lifecycle.v1`;
- `phase`: `starting`, `connecting`, `online`, `degraded`, `stopping`,
  `terminal_failure`, or `stopped`;
- `ready`: true only for `online`;
- `process_active`: whether the foreground process currently owns the dedicated
  lifecycle lock;
- `updated_at` and `pid` when a snapshot exists;
- `failure_kind`: bounded lowercase category, never raw error text;
- `last_observed_phase`: stale active phase retained only when the process is no
  longer alive.

This local contract does not replace `remote-session status`. The latter reads
backend-authoritative provider-session state. Local `online` means that this
foreground process completed the authenticated WebSocket handshake and received
`session_ready`; it does not mean provider verification, workload eligibility,
listing availability, lease activity, or marketplace approval.

## Persistence And Crash Semantics

The Agent writes `agent-lifecycle.json` atomically in its state directory and
holds `agent-lifecycle.lock` for the foreground process lifetime. If the process
crashes, the operating system releases the lock. A reader that sees a persisted
active phase without the lock returns effective state `stopped`, with
`ready=false`, `process_active=false`, `failure_kind=process_not_running`, and
the persisted phase in `last_observed_phase`.

Lifecycle files contain no private key, enrollment credential, resume token,
authorization header, or raw backend error. The reader rejects symbolic links,
non-files, unsupported schema versions, inconsistent readiness, invalid failure
tokens, and files above 16 KiB.

## Cooperative Shutdown

Shutdown now races:

- connection preparation;
- WebSocket connection;
- the initial `session_ready` response;
- retry sleeps;
- active credential refresh;
- the established control loop;
- the existing Proof of Capability worker.

Connection preparation checks cancellation around enrollment loading, credential
freshness, hardware registration, session start/resume, and local persistence.
The supervisor requests cancellation and waits up to five seconds for blocking
startup or refresh work.

Rust cannot force-stop an already running `spawn_blocking` closure. Native
hardware/vendor calls and blocking HTTP requests already in progress may
continue until they return or hit their own timeout. Aborting the Tokio join
handle after the grace period stops supervisor waiting; it does not terminate
the underlying blocking operation or establish a global process-exit deadline.

## Tests

Unit coverage verifies:

- valid lifecycle transitions;
- readiness only in `online`;
- sensitive fields absent from persisted state;
- OS lock ownership controls `process_active`;
- a stale `online` snapshot is never reported ready;
- late work cannot overwrite a terminal phase;
- shutdown cancels deterministic active connection preparation.

The ignored real-infrastructure harness verifies:

- initial `online` lifecycle after acknowledged heartbeat;
- socket loss and session resume;
- `degraded` during Control Plane unavailability;
- recovery to `online`;
- expired-session replacement after a prolonged outage;
- `terminal_failure` after backend revocation;
- clean shutdown reaches `stopped`;
- all session behavior persists through isolated PostgreSQL.

## Validation

Commands run:

```powershell
cargo fmt --all --check
cargo clippy -p burd-agent --all-targets --features integration-test-support --no-deps -- -D warnings
cargo test -p burd-agent
cargo test --workspace
cargo build --workspace
$env:BURD_AGENT_CONFIG='C:\tmp\burd-agent-lifecycle-cli\agent.json'
.\target\debug\burd-agent.exe remote-session lifecycle --json
$env:BURD_CONTROL_TEST_DATABASE_URL='postgres://burd:burd@127.0.0.1:5432/burd_test'
cargo test -p burd-control-plane -- --ignored
```

Results:

- formatting, Agent Clippy, workspace tests, and workspace build passed;
- the workspace executed 260 passing tests and left 25 environment-dependent
  tests ignored by default;
- the focused Agent run executed 34 passing tests and left one live Ollama test
  ignored;
- all 20 Control Plane PostgreSQL tests and all 3 Agent/WebSocket integration
  tests passed against the isolated Docker database;
- the isolated lifecycle CLI smoke test returned `stopped`, `ready=false`, and
  `process_active=false` with no persisted state;
- two pre-existing `third_party/llmfit` dead-code warnings remain outside this
  change;
- no physical NVIDIA/CUDA/Ollama matrix was run.

## Remaining Limits

- The Agent remains a foreground command.
- No Windows Service or systemd packaging exists.
- `remote-session connect` has a stable typed exit taxonomy; other commands still use legacy code `1`.
- Blocking native/HTTP work is cooperative only at explicit boundaries.
- WebSocket close and total process shutdown have no single global deadline.
- Physical NVIDIA/CUDA/Ollama stop/restart matrices remain environment work.
