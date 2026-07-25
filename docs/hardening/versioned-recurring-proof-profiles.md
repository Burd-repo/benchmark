# Versioned Recurring Proof Profiles

## Summary

This pass closes the execution gap between the BN-07 recurring verification
sweep and the BN-06 Agent proof runner. The sweep no longer creates challenges
with the non-executable `sha256:burd-poc-v1` placeholder and zero performance
thresholds.

Recurring verification is now explicitly disabled until the Control Plane has a
complete deployment profile. A configured sweep snapshots the profile version,
exact Ollama model digest, canonical proof requirements, minimum tokens per
second, and maximum TTFT into each challenge.

## Scope

Changed:

- Control Plane startup configuration and validation;
- BN-07 sweep challenge construction;
- the shared BN-06 proof-name contract;
- Agent/backend use of that shared contract;
- HTTP/OpenAPI failure behavior when recurrence is disabled;
- unit and isolated PostgreSQL coverage;
- BN-07 and current-state documentation.

Unchanged:

- signed BN-06 challenge and response schemas;
- database schema and applied migrations;
- manual admin challenge issuance;
- Agent CUDA, cuBLAS, Ollama, telemetry, hashing, and signing execution;
- verification state transitions and trust consumers.

## Configuration Contract

The deployment profile uses:

- `BURD_CONTROL_VERIFICATION_PROFILE_VERSION`;
- `BURD_CONTROL_VERIFICATION_MODEL_ARTIFACT_HASH`;
- `BURD_CONTROL_VERIFICATION_REQUIRED_PROOFS`;
- `BURD_CONTROL_VERIFICATION_MIN_TOKENS_PER_SECOND`;
- `BURD_CONTROL_VERIFICATION_MAX_TTFT_MS`.

`MODEL_ARTIFACT_HASH` must be an exact `sha256:<64 hex>` digest. The proof list
must contain every canonical BN-06 proof exactly once. TPS and TTFT thresholds
must both be positive when a digest is configured.

With no digest and zero thresholds, the backend starts with recurring proof
issuance disabled. Partial or malformed profiles fail startup. An authenticated
sweep against a disabled profile returns the BN-00 `invalid_request` envelope
with HTTP `400` before database access.

## Contract Cleanup

The canonical proof names now live in `burd-protocol`. The Agent and Control
Plane both consume that exported list, removing duplicated constants that could
drift and create challenges the Agent could not execute.

The immutable `proof_challenges` row already stores all profile fields. No new
migration or mutable profile foreign key was needed; historical challenges keep
the exact requirements under which they were issued.

## Tests Added

- default startup leaves recurring proof issuance disabled;
- a complete versioned profile is normalized and accepted;
- partial profiles, placeholder digests, and incomplete proof sets are rejected;
- challenge construction preserves every configured profile field;
- HTTP sweep fails closed with the standard error envelope while disabled;
- an ignored PostgreSQL test enrolls a provider, starts an online session,
  submits signed telemetry, runs a configured sweep, and verifies the API and
  persisted challenge/state fields.

## Validation

Executed during implementation:

```powershell
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
$env:BURD_CONTROL_TEST_DATABASE_URL='postgres://burd:burd@127.0.0.1:5432/burd_test'
cargo test -p burd-control-plane -- --ignored
cargo clippy --workspace --all-targets
```

All commands passed. The normal workspace suite ran 233 tests with one
intentional slow hardware test ignored. The PostgreSQL command ran all 23
ignored database/integration tests: 20 Control Plane tests and three Agent
WebSocket harnesses.

Clippy reported only existing warnings in `burd-bench`, `billing.rs`,
`gpu_inventory.rs`, and `third_party/llmfit`; none originated from this pass.

## Remaining Limitations

- Profile selection is deployment-wide, not per GPU family, region, or provider.
- The Control Plane does not distribute or prefetch the configured model.
- The sweep remains admin-triggered; there is no autonomous challenge scheduler.
- Physical CUDA/Ollama execution was not performed by this test pass.
- The Agent still runs in the foreground and keeps proof attempt failure history
  in memory.
- This is signed remote observation under the current software key boundary, not
  hardware-backed remote attestation.

## Recommended Next Work

1. Run a controlled physical-GPU compatibility matrix for CUDA UUID binding,
   VRAM residency, cuBLAS, Ollama digest matching, and contention behavior.
2. Add Agent service supervision and durable bounded proof-attempt state.
3. Add artifact distribution and profile selection only after the single-profile
   operational path has production evidence.