# Agent Exit Status Contract

Date: 2026-07-27

## Summary

This hardening pass defines stable, machine-readable process exit semantics for
the foreground `burd-agent remote-session connect` command. Terminal supervisor
failures now retain a typed category from the remote-session boundary to the CLI,
and the CLI writes a redacted `burd.agent.exit.v1` event to stderr.

This work does not add a daemon, Windows Service, systemd unit, restart policy,
service manager integration, or new remote protocol.

## Stable Codes

| Category | Code | Meaning |
| --- | ---: | --- |
| `operator_requested` | 0 | Foreground shutdown was requested and completed |
| `invalid_invocation` | 2 | Remote-session semantic arguments are invalid |
| `local_state` | 10 | Required local identity/session/lifecycle state is invalid or unavailable |
| `unauthorized` | 11 | Credentials are invalid, expired, or rejected |
| `revoked` | 12 | Device or session was revoked by the Control Plane |
| `remote_rejected` | 13 | Non-retryable Control Plane rejection |
| `remote_contract` | 14 | Control Plane response violated the expected contract |
| `internal` | 15 | Runtime, task, or proof-worker failure |

These numeric assignments are part of `burd.agent.exit.v1` and must not be
reused for different meanings.

Recoverable transport failures, HTTP `408`, `429`, `5xx`, conflicts, socket
loss, and temporary Control Plane unavailability do not exit. The Agent moves
to local lifecycle `degraded`, applies bounded backoff, and keeps reconnecting.

## Event Contract

Typed exits write one JSON object to stderr:

```json
{
  "schema_version": "burd.agent.exit.v1",
  "event": "agent_exit",
  "category": "revoked",
  "exit_code": 12,
  "failure_kind": "session_revoked",
  "message": "Agent device or session was revoked by the Control Plane."
}
```

The event contains only a fixed operator message and a bounded internal failure
kind. It never includes the private diagnostic detail, credentials, resume
tokens, authorization headers, private keys, or backend response bodies.
Both `Debug` and `Display` output for the typed error omit private diagnostic
detail. The detail remains inside the typed Rust error behind the explicit
`diagnostic_detail()` accessor for controlled local diagnostics and tests. The
foreground CLI detects the type through the `anyhow` error chain and prints
only the redacted event.

## Lifecycle Consistency

When startup fails before the control loop becomes online, the lifecycle
snapshot now records the typed failure kind instead of the generic
`session_runtime` marker. Backend revocation produces:

- lifecycle `phase=terminal_failure`;
- lifecycle `failure_kind=session_revoked`;
- exit category `revoked`;
- process exit code `12`.

An operator-requested stop produces lifecycle `stopped`, exit category
`operator_requested`, and code `0`. If shutdown occurs before a remote session
is persisted, the command still exits successfully and does not fabricate a
session status payload.

## Compatibility Boundaries

- Existing successful remote-session status JSON remains unchanged when a
  persisted session exists.
- Other Agent commands still use legacy untyped code `1` and their existing
  human-readable error output.
- Clap-owned syntax and parsing errors still use Clap's code `2` and native
  diagnostic format; they do not emit `burd.agent.exit.v1`.
- The typed `invalid_invocation` event currently covers semantic
  remote-session validation such as an invalid telemetry batch size.
- No retry limit or temporary-outage exit was introduced.

## Tests

Coverage verifies:

- every category keeps its assigned numeric code;
- terminal failure kinds map to the expected category;
- structured events and `Debug` output omit injected secret values;
- actual CLI processes return code `2` for semantic invalid invocation;
- actual CLI processes return code `10` for missing local identity;
- startup lifecycle state preserves `failure_kind=local_state`;
- WebSocket `400`, `401`, and `403` remain distinct contract, authorization,
  and revocation failures;
- the real PostgreSQL/WebSocket revocation flow returns category `revoked`,
  code `12`, and `failure_kind=session_revoked`.

## Validation

Executed on 2026-07-27:

- `cargo fmt --all --check`: passed.
- `cargo clippy -p burd-agent --all-targets --features integration-test-support --no-deps -- -D warnings`:
  passed for the Agent. Cargo still reports two pre-existing warnings from
  `third_party/llmfit`.
- `cargo test -p burd-agent`: 40 passed and 1 live Ollama test ignored.
- `cargo test --workspace`: 266 passed and 25 ignored by the default test
  policy.
- `$env:BURD_CONTROL_TEST_DATABASE_URL='postgres://burd:burd@127.0.0.1:5432/burd_test'; cargo test -p burd-control-plane -- --ignored`:
  20 PostgreSQL Control Plane tests and 3 Agent/WebSocket integration tests
  passed against the isolated `burd_test` database.
- `cargo build --workspace`: passed.

Of the 25 tests ignored in the default workspace run, the 23 PostgreSQL and
Agent/WebSocket tests were executed separately and passed. The live Ollama test
and slow physical hardware-detection test were not executed because this
validation did not provide their required local model and hardware matrix.

## Remaining Limits

- The Agent remains a foreground command.
- Untyped commands still use legacy code `1`.
- Clap syntax failures do not use the structured event.
- No service manager consumes these codes yet.
- Blocking native/HTTP work still cannot be force-cancelled.
- Physical Windows/Linux service and NVIDIA/CUDA/Ollama matrices remain
  environment work.
