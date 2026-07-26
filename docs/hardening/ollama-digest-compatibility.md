# Ollama Digest Compatibility

## Summary

The first physical-compatibility audit found an executable BN-06 contract bug.
The Control Plane correctly stores model artifacts as `sha256:<64 hex>`, while
Ollama `/api/tags` reports the same digest as 64 hexadecimal characters without
the algorithm prefix. The Agent compared those strings directly, so a recurring
challenge configured after the versioned-profile pass could not select any
installed model.

The Agent now canonicalizes both representations to lowercase 64-hex SHA-256
before exact comparison. This changes representation handling only: shortened
Ollama IDs, non-SHA-256 algorithms, malformed values, and different digests are
still rejected.

## Scope

Changed:

- Ollama model inventory parsing and selection in the Agent proof executor;
- unit coverage for canonical and invalid digest forms;
- one ignored test that performs real local Ollama inference;
- BN-06 and current-state documentation.

Unchanged:

- the backend-issued challenge schema;
- the canonical `sha256:<64 hex>` profile value stored by the Control Plane;
- signed response hashing and Ed25519 binding;
- CUDA, VRAM, cuBLAS, telemetry, and backend verification behavior;
- database schema and migrations.

## Root Cause

On the audited host, `/api/tags` returned full model digests such as:

```text
a80c4f17acd55265feec403c7aef86be0c25983ab279d83f3bcd3abbcb5b8b72
```

The BN-07 profile contract requires:

```text
sha256:a80c4f17acd55265feec403c7aef86be0c25983ab279d83f3bcd3abbcb5b8b72
```

Both identify the same artifact, but direct string equality treated them as
different.

## Validation Matrix

| Check | Result on this host | Evidence |
| --- | --- | --- |
| Full SHA-256 digest normalization | Passed | Unit tests cover raw/uppercase inventory values, canonical prefixed challenges, malformed or shortened hashes, alternate algorithms, and mismatches. |
| Exact installed-model binding | Passed | The ignored live test resolved a prefixed challenge digest against `/api/tags`. |
| Real short Ollama inference | Passed | The live test generated tokens and observed finite TPS and positive TTFT. |
| CUDA driver/runtime loading | Not executed | The host reports an AMD Radeon RX 5700 XT and `cuda_available: false`. |
| NVIDIA telemetry/CUDA UUID binding | Not executed | No NVIDIA GPU or `nvidia-smi` is available on this host. |
| CUDA VRAM residency | Not executed | Requires a physical NVIDIA CUDA device. |
| cuBLAS SGEMM | Not executed | Requires CUDA and cuBLAS on a physical NVIDIA host. |
| Physical contention behavior | Not executed | Requires a controlled competing NVIDIA workload. |

The Ollama result is a real local runtime compatibility check. It is not remote
verification, hardware attestation, or evidence that the CUDA proof completed.

## Tests

Added:

- `model_binding_accepts_prefixed_challenge_and_raw_ollama_digest`;
- `model_binding_rejects_short_invalid_and_different_digests`;
- ignored `live_ollama_inference_binds_prefixed_inventory_digest`.

The live test is ignored by default because CI cannot assume a running Ollama
service or installed model. Run it explicitly with:

```powershell
cargo test -p burd-agent live_ollama_inference_binds_prefixed_inventory_digest -- --ignored --nocapture
```

`OLLAMA_HOST` continues to override the default local endpoint.

## Validation

Executed:

```powershell
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
cargo test -p burd-agent live_ollama_inference_binds_prefixed_inventory_digest -- --ignored --nocapture
$env:BURD_CONTROL_TEST_DATABASE_URL='postgres://burd:burd@127.0.0.1:5432/burd_test'
cargo test -p burd-control-plane -- --ignored
cargo clippy -p burd-agent --all-targets
```

All commands passed. The workspace suite ran 235 normal tests; the explicit
Ollama runtime test and the existing slow hardware detector are ignored by
default. The PostgreSQL suite ran 23 ignored tests: 20 Control Plane tests and
three Agent/WebSocket harnesses.

Clippy reported only pre-existing warnings in `burd-bench` and
`third_party/llmfit`; none originated from the changed Agent module.

## Remaining Work

1. Run the existing production CUDA executor on controlled NVIDIA hosts covering
   supported Windows/Linux drivers, CUDA runtimes, and GPU families.
2. Record CUDA/NVIDIA UUID agreement, telemetry while residency is held, cuBLAS
   output, model digest, performance metrics, and contention classification.
3. Keep physical results outside normal CI and label them as local diagnostics,
   never backend verification or attestation.
4. After the CUDA matrix, add Agent service supervision and durable bounded proof
   attempt/error state.