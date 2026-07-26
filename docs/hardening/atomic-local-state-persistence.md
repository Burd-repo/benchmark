# Atomic Local State Persistence

Date: 2026-07-26

## Summary

Canonical Agent JSON state now uses one shared per-file atomic persistence
primitive. Writers serialize before touching the destination, create a unique
temporary file in the same directory, flush its contents, and atomically replace
the destination.

This hardening prevents readers from observing truncated or partially written
JSON after interruption or concurrent reads. It does not change JSON fields,
signed payloads, hashes, CLI output, HTTP contracts, or PostgreSQL schema.

## Shared Primitive

`burd-protocol::write_json_atomic` and `write_bytes_atomic` provide the shared
implementation:

- the destination parent directory is created when missing;
- serialization finishes before a temporary file is opened;
- the temporary file uses unpredictable entropy and `create_new`;
- existing file permissions are copied to the replacement;
- Windows DACL entries and protected/inherited DACL state are preserved;
- file contents are synchronized before replacement;
- the temporary file is created beside the destination so replacement stays on
  the same filesystem;
- Unix synchronizes the parent directory after rename;
- Windows uses replacement with write-through semantics and bounded retry for
  transient sharing or access conflicts;
- failed writes remove their temporary file when possible;
- symbolic links and non-regular destinations fail closed.

The Windows-specific dependency is target-gated and does not affect non-Windows
builds.

## Migrated Canonical State

The following persisted state now uses the shared primitive:

- Agent identity configuration and private key file;
- remote enrollment and short-lived credential state;
- local provider session state;
- remote session resume and sequence state;
- latest local challenge response;
- latest unsigned and signed reports;
- benchmark history, including clear;
- local action state;
- uptime history;
- latest local network benchmark;
- bounded remote Proof of Capability attempt history.

The existing maintenance lock still serializes identity, enrollment, credential,
API-token, and foreground remote-session operations. Atomic replacement also
protects unlocked readers and independent diagnostic state files from partial
content.

## Deliberate Exclusions

Explicit user-selected exports remain direct writes:

- benchmark history export;
- provider registration payload export;
- local provider session export.

Those paths are output artifacts selected by the caller, not canonical Agent
state. Disk benchmark scratch files and test fixtures also remain unchanged.

## Compatibility

The serialized bytes remain `serde_json` pretty JSON. No schema version,
canonicalization rule, signature domain, report hash, response field, CLI
command, API route, OpenAPI schema, database migration, or backend behavior
changed.

Replacing a canonical state path with a symbolic link is now rejected instead
of following the link. This is an intentional local safety hardening.

## Tests

The shared primitive is tested for:

- replacement of existing JSON and temporary-file cleanup;
- preservation of the previous file when serialization fails;
- concurrent reads during repeated large writes without partial JSON;
- rejection of non-file destinations;
- preservation of a protected Windows DACL;
- rejection of symbolic-link destinations on Unix;
- preservation of existing Unix file mode.

Existing contract and state tests cover all migrated writers without snapshot
changes.

## Remaining Limitations

- Atomicity is per file. Identity key and configuration updates are not one
  multi-file transaction.
- Atomic replacement does not prevent lost updates when two independent
  processes perform read-modify-write on the same history or action file.
- The maintenance lock is not applied to every benchmark or diagnostic command.
- The destination inspection and replacement are not a hardened filesystem
  sandbox against a hostile local administrator.
- File durability ultimately depends on operating-system and filesystem
  guarantees.
- Private-key storage is still a local file, not TPM, HSM, or operating-system
  keychain storage.
- The Agent remains a foreground process without Windows Service or systemd
  packaging.

## Validation

Run for this hardening pass:

```powershell
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
cargo test -p burd-protocol -p burd-bench -p burd-agent
cargo clippy -p burd-protocol --all-targets
cargo clippy -p burd-agent --all-targets
$env:BURD_CONTROL_TEST_DATABASE_URL='postgres://burd:burd@127.0.0.1:5432/burd_test'
cargo test -p burd-control-plane -- --ignored
```

Results:

- `cargo fmt --all --check`: passed;
- `cargo build --workspace`: passed;
- `cargo test --workspace`: 252 passed and 25 environment-dependent tests
  ignored;
- focused Agent, benchmark, and protocol suite: 124 passed and 2
  environment-dependent tests ignored;
- protocol and Agent Clippy: passed; existing warnings remain in
  `third_party/llmfit` and pre-existing `burd-bench` code;
- PostgreSQL ignored suite: 20 Control Plane tests and 3 Agent/WebSocket
  integration tests passed against `burd-postgres-test`.

A workspace invocation made through the restricted test token could not mutate a
Windows DACL and was discarded; the required direct `cargo test --workspace`
command passed, including the protected-DACL integration test. Initial nested
PowerShell attempts also omitted the PostgreSQL variable because of shell
quoting; no database test ran in those attempts, and the corrected command above
passed all 23 database-backed tests.

The PostgreSQL suite must use the isolated local test database. The live Ollama
test remains environment-dependent and is not evidence of physical GPU
compatibility unless a real local model service is used.

## Recommended Next Work

1. Freeze Agent service startup, shutdown, recovery, credential refresh, and
   update policy before adding operating-system service packaging.
2. Add explicit coordination only where a demonstrated read-modify-write race
   exists; do not turn every diagnostic command into a global critical section.
3. Run the BN-06 physical CUDA/Ollama compatibility matrix on controlled NVIDIA
   Windows and Linux hosts.
