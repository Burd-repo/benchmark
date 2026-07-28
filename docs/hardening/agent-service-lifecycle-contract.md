# Agent Service Lifecycle Contract

Date: 2026-07-27

## Summary

This hardening pass freezes the lifecycle contract of the existing foreground
`burd-agent remote-session connect` process before any Windows Service or
systemd packaging is added. It fixes local session recovery and authority
binding, documents startup, retry, credential refresh, shutdown, and crash
recovery, and records the remaining service-readiness gates.

This work does not add a daemon, installer, autostart, background updater,
scheduler worker, job runtime, or new remote protocol.

## Implemented Lifecycle

### Startup

1. Validate CLI telemetry settings.
2. Acquire the exclusive Agent state-directory lock.
3. Load the local identity and derive the deterministic retry seed.
4. Start the foreground Tokio runtime and install the `Ctrl+C` shutdown signal.
5. Before every connection attempt, refresh a credential that expires within
   two minutes, collect the registration snapshot, and start or resume the
   remote session.
6. Open the authenticated outbound WebSocket and start heartbeats, optional
   signed telemetry, and the optional BN-06 proof worker.

Identity loading, credential refresh, hardware registration, local signing, and
proof execution use blocking tasks so they do not block Tokio worker threads.
Connection preparation and active credential refresh receive a cooperative
shutdown token and check it before and after blocking boundaries. A native or
HTTP call already in progress cannot be force-cancelled.

### Session Authority And Recovery

- A missing `remote-session.json` means there is no resumable local session and
  the Agent requests a new backend session.
- An unreadable or malformed session file is a local-state failure. The Agent
  stops instead of silently creating a second backend session.
- A persisted session is resumable only when its normalized Control Plane URL
  matches the current enrollment. State from another Control Plane is removed
  before a request is sent, so its resume token is never disclosed to the new
  authority.
- Every successful enrollment invalidates prior local session state before the
  new enrollment is persisted.
- Backend `404`, `410`, `not_found`, or `expired` responses for a persisted
  session clear it and allow one fresh session request.
- Transport, HTTP `408`, `429`, `5xx`, and conflict failures retry with bounded
  exponential backoff and deterministic per-agent jitter.
- Invalid credentials, revocation, malformed contracts, and unrecoverable local
  state stop the foreground command.
- Retry counters are process-local and reset after restart. A heartbeat
  acknowledgement resets the failure series because it proves the connection
  became usable.

### Credential Refresh

- The Agent checks credential expiry before each connection attempt.
- A credential within two minutes of expiry is refreshed before session
  start/resume.
- A long-lived connection repeats the same check after control-channel work.
- Refresh persists the new short-lived credential atomically in
  `remote-enrollment.json`.
- The foreground connection owns the state lock while refreshing, so a separate
  enrollment or manual refresh process cannot race it.
- Backend device revocation remains authoritative and stops refresh or
  connection attempts.

### Graceful Shutdown

- `Ctrl+C` and the integration-test shutdown channel use the same internal
  signal.
- Shutdown interrupts retry sleeps and the active control loop.
- Shutdown races WebSocket connection and `session_ready` waits.
- The control channel attempts a WebSocket close and stops heartbeat and
  telemetry production.
- The supervisor signals the proof worker and waits for both control and proof
  tasks before returning.
- An active proof now observes the same shutdown signal while waiting for
  readiness, telemetry, or executor completion. The supervisor requests
  cancellation, releases the telemetry gate, and waits up to five seconds for
  cooperative proof completion.
- Connection preparation and active credential refresh use the same pattern:
  request cooperative cancellation and wait up to five seconds for their
  blocking task.
- The state lock is released when the foreground process exits.

Graceful shutdown is not yet bounded in every phase. Connection preparation,
credential refresh, and active CUDA/Ollama proof check cancellation between
expensive operations, but a blocking vendor/runtime or HTTP call already in
progress can continue until it returns or reaches its existing timeout. The
five-second grace period bounds supervisor waiting, not native work or total
process exit time.

### Crash Recovery

- Canonical identity, enrollment, session, proof-attempt, report, history, and
  related local JSON files use atomic replacement per file.
- The operating system releases the process lock after a crash; the persistent
  lock file is metadata, not evidence that the process is alive.
- `agent-lifecycle.json` is an atomic local snapshot. A separate
  `agent-lifecycle.lock` is held for the lifetime of the foreground process.
  Readers that find a persisted active phase without that OS lock report
  `stopped`, `ready=false`, `process_active=false`, and preserve the stale phase
  only as `last_observed_phase`.
- On restart, the Agent reloads identity and enrollment, re-evaluates credential
  freshness and hardware registration, and resumes only a valid session bound to
  the same Control Plane.
- Control and telemetry sequence numbers persist after backend
  acknowledgements.
- Atomic replacement is not a multi-file transaction. A crash between
  enrollment session invalidation and enrollment persistence can leave no local
  session, which is a recoverable and conservative state.

## Update Policy Freeze

The repository has no automatic Agent updater and must not imply otherwise.
Future update work must, at minimum, provide:

- signed release artifacts and verified publisher identity;
- explicit stable/test release channels;
- protocol and local-state compatibility checks;
- atomic installation with rollback;
- a defined policy for active proof or paid job execution;
- redacted update audit events;
- tested Windows and Linux recovery from interrupted updates.

Until those controls exist, Agent updates remain an external/manual deployment
operation.

## Security Findings And Fixes

- Fixed: corrupt local session JSON was previously treated as missing through
  `.ok()`, which could create an untracked second backend session.
- Fixed: a resume token persisted for one Control Plane could previously be
  attached to a start request sent to a different enrolled Control Plane.
- Fixed: successful enrollment previously retained the prior local resume token.
- Preserved: private keys, device credentials, resume tokens, and authorization
  headers are not added to lifecycle or retry logs.
- Added: lifecycle failures persist only a bounded lowercase category token;
  error text, credentials, and request headers are not persisted.
- Preserved: backend session status, credential revocation, and device
  revocation remain authoritative.

## Tests

Added or extended coverage verifies:

- missing session state is distinct from malformed session state;
- status views continue to redact the resume token;
- trailing slash normalization preserves same-authority resume;
- different Control Plane state is rejected for resume;
- successful live enrollment removes stale local session credentials before the
  Agent connects;
- the existing live PostgreSQL/WebSocket flow still covers heartbeat,
  reconnect, backend outage, resume, expiry replacement, and revocation;
- lifecycle snapshots move through online, degraded, recovered online,
  terminal failure, and clean stopped states;
- shutdown cancels a deterministic active connection-preparation task without
  waiting for its full synthetic workload.

## Service Packaging Gates

Before adding Windows Service or systemd packaging:

1. Extend cooperative cancellation into native/vendor and HTTP clients where
   their APIs support interruption. Startup, credential refresh, and proof
   supervisors now request cancellation and use a five-second grace period, but
   in-flight blocking calls are not force-cancellable.
2. Keep the implemented `burd.agent.exit.v1` remote-session categories stable
   and migrate other commands only when their error boundaries are typed.
   Recoverable outage intentionally remains `degraded` and does not exit.
3. Keep the implemented local lifecycle/readiness contract stable and integrate
   it with future service-manager health checks. It distinguishes starting,
   connecting, online, degraded, stopping, terminal failure, and stopped.
4. Define service-account permissions for identity keys, enrollment state,
   object artifacts, logs, and GPU/runtime access.
5. Implement the signed update and rollback policy above.
6. Run stop, crash, reboot, network outage, credential expiry, and physical
   NVIDIA compatibility matrices on supported Windows and Linux versions.

## Validation

Commands run:

```powershell
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
cargo test -p burd-protocol --lib
cargo test -p burd-agent --lib
cargo test -p burd-control-plane --test agent_remote_session --no-run
cargo clippy -p burd-protocol --all-targets
cargo clippy -p burd-agent --all-targets
$env:BURD_CONTROL_TEST_DATABASE_URL='postgres://burd:burd@127.0.0.1:5432/burd_test'
cargo test -p burd-control-plane -- --ignored
```

Results:

- format and workspace build passed;
- workspace tests passed with 260 tests and 25 environment-dependent tests
  ignored;
- focused protocol tests passed with 36 tests;
- focused Agent library tests passed with 32 tests and one live Ollama test
  ignored;
- the Agent remote-session PostgreSQL harness compiled;
- protocol and Agent Clippy passed; pre-existing warnings remain in
  `third_party/llmfit` and `burd-bench`;
- 20 Control Plane PostgreSQL tests and all 3 Agent/WebSocket integration tests
  passed against the isolated `burd-postgres-test` database.

An initial focused invocation under the restricted test token could not mutate a
protected Windows DACL. It was not counted as a valid test result. The required
direct `cargo test --workspace` invocation passed that DACL test. The live
Ollama test was not run because it requires a local service with an installed
model and does not replace the physical NVIDIA compatibility matrix.

## Remaining Limitations

- The Agent is still a foreground command, not an operating-system service.
- Startup preparation, credential refresh, and active proof execution check
  cooperative cancellation at explicit boundaries. In-flight native or
  blocking HTTP calls may continue until they return; each five-second grace
  period only bounds supervisor waiting.
- WebSocket close does not yet have a global process shutdown deadline.
- The remote-session foreground command exposes stable typed exit categories,
  but other commands still use legacy code `1`, Clap syntax failures keep native
  code `2`, and no service manager consumes the contract yet.
- Retry history is not durably exposed to a service manager.
- No automatic update, release signature verification, rollback, or installer
  exists.
- Physical CUDA/Ollama compatibility remains dependent on controlled hardware
  validation.
