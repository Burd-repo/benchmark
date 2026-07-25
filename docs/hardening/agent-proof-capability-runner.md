# Agent Remote Proof Of Capability Runner

Date: 2026-07-25

## Summary

This pass implements the missing Agent side of BN-06 without introducing a
background daemon or a general-purpose remote execution surface. The foreground
remote-session command can opt into backend-issued, approved CUDA capability
proofs and submit a signed response linked to telemetry accepted by the Control
Plane.

## Scope

Audited and changed:

- Agent remote-session CLI and WebSocket control loop;
- challenge pickup, local validation, execution, hashing, signing, and response
  submission;
- NVIDIA telemetry capture during the proof execution window;
- CUDA runtime, VRAM residency, cuBLAS SGEMM, and Ollama inference boundaries;
- PostgreSQL integration coverage for the complete Agent-to-Control-Plane flow;
- BN-06, BN-07, README, and current-state documentation.

No migrations, backend API contracts, marketplace behavior, jobs, scheduler,
billing, Pix, payouts, or general workload runtime were added.

## Implemented Behavior

- `burd-agent remote-session connect --proofs` enables the worker and implies
  signed telemetry.
- The worker polls the authenticated session challenge endpoint every five
  seconds and accepts only the seven BN-06 proof names already defined by the
  protocol.
- Each challenge is bound to the active provider, device, session, server expiry,
  CUDA backend, current hardware fingerprint, optional GPU UUID, artifact hash,
  prompt seed, and thresholds before execution.
- A dynamic `cudarc` integration loads driver/runtime/cuBLAS libraries at runtime,
  selects a GPU whose CUDA UUID matches NVIDIA telemetry, allocates real VRAM,
  and measures SGEMM throughput.
- LLM proof execution uses Ollama streaming generation, requires an exact model
  digest match, binds nonce and prompt seed into the prompt, measures wall-clock
  TTFT, and uses Ollama evaluation counters for token throughput.
- The CUDA allocation remains resident while the single WebSocket writer submits
  a fresh signed telemetry sample. Execution continues only after the backend
  acknowledges that batch.
- The response uses existing canonicalization, response hashing, Ed25519 signing,
  and active enrollment key contracts.

## Bugs And Risks Addressed

- The first PostgreSQL run correctly failed because fixture telemetry represented
  unexplained GPU utilization. The proof fixture now uses an idle sample while
  the existing telemetry harness retains its 42 percent assertion.
- The worker originally captured one startup fingerprint. It now recalculates
  hardware immediately before each challenge, preventing execution after an
  unreflected hardware change.
- Forced proof telemetry now clears only unsubmitted local samples and sends a
  fresh single-sample window, preventing older buffered samples from being used
  as execution-time evidence.
- Startup before `remote-session.json` exists is treated as a session-not-ready
  condition instead of a misleading proof-fetch failure.
- Heartbeats and proof telemetry share one WebSocket writer, preserving monotonic
  control sequences and acknowledgement handling.
- Unknown proof names, stale challenges, backend mismatch, GPU mismatch, missing
  dependencies, digest mismatch, telemetry rejection, and response timing errors
  fail explicitly. Metrics are never synthesized.

## Test Design

The ignored PostgreSQL test uses a deterministic executor behind
`integration-test-support`. That executor is not reachable from the production
CLI. It replaces only physical compute and telemetry collection; enrollment,
WebSocket transport, sequence management, canonical telemetry signing,
PostgreSQL persistence, challenge pickup, response signing, object persistence,
and backend verification remain production paths.

The test verifies:

- final `verified` challenge status;
- nonce/provider/device/session/fingerprint/GPU bindings;
- response hash and Ed25519 signature;
- all backend binding and metric verification flags;
- no verification errors;
- a registered telemetry window for the same session;
- exactly one fresh telemetry sample in the proof-linked batch.

## Commands Executed

Passed during implementation and final validation:

```powershell
cargo check -p burd-agent --all-targets
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cargo test -p burd-control-plane
cargo test -p burd-agent
cargo test -p burd-api-local
cargo test -p burd-bench
cargo test -p burd-protocol
cargo test -p burd-hardware
cargo test -p burd-llmfit
cargo test -p burd-control-plane --test agent_remote_session --no-run
cargo clippy -p burd-agent --all-targets
cargo run -p burd-agent -- remote-session connect --help
$env:BURD_CONTROL_TEST_DATABASE_URL='postgres://burd:burd@127.0.0.1:5432/burd_test'
cargo test -p burd-control-plane -- --ignored
```

The normal workspace suite passed with 227 tests, one intentionally ignored slow
hardware-detection test, and no failures. The isolated PostgreSQL suite passed
all 22 ignored database tests: 19 Control Plane tests and three Agent harnesses.

One intentional strict-Clippy attempt failed on pre-existing `burd-bench`
warnings before reaching the Agent:

```powershell
cargo clippy -p burd-agent --all-targets -- -D warnings
```

Normal Clippy completed successfully. It reports existing `burd-bench` style
warnings and two inherited `third_party/llmfit` dead-code warnings; none originate
from the new Agent proof modules.

## Validation Not Performed

This pass did not execute the production CUDA/Ollama executor on physical GPU
hardware. The machine-independent integration harness must not be interpreted as
hardware attestation or a real performance result.

## Remaining Limitations

- The command remains foreground-only and has no service supervision.
- Local failure history and attempted-challenge suppression are in memory only.
- A local execution failure is logged and left for backend expiry; there is no
  signed failure-response contract.
- A later hardening pass made BN-07 recurrence fail closed and require a real,
  versioned deployment profile; artifact distribution and per-GPU selection
  remain open. See `versioned-recurring-proof-profiles.md`.
- Ollama is the only LLM runtime supported by this first executor.
- CUDA compatibility still requires slow tests across supported driver/runtime
  versions and GPU families.
- The proof establishes signed observation and fresh work under the current
  software trust boundary; it is not TPM-backed remote attestation.

## Recommended Next PRs

1. Completed by `versioned-recurring-proof-profiles.md`: recurring verification
   now requires a real, versioned artifact digest and positive thresholds.
2. Add a controlled physical-GPU test matrix for CUDA library loading, UUID
   binding, residency, SGEMM, Ollama digest binding, and contention behavior.
3. Add Agent service supervision and durable proof attempt/error state without
   changing the signed BN-06 response contract.