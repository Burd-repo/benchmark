# BN-06 - Active Proof Of Capability Protocol

BN-06 defines the backend-issued Proof of Capability protocol, its authoritative
verifier, and the foreground Burd Agent runner. Recurring verification state is
implemented by BN-07. This scope does not add scheduler decisions, jobs,
marketplace, billing, or a supervised Agent daemon.

## Scope

Implemented:

- backend-issued proof challenges for an online or degraded remote session;
- backend-attested `challenge_id`, nonce, issue time, expiry, required
  fingerprint, optional GPU UUID, backend, artifact hash, prompt seed,
  thresholds, and required proof names;
- session-authenticated challenge pickup by the enrolled device;
- `burd-agent remote-session connect --proofs`, which polls for challenges while
  maintaining the authenticated outbound WebSocket session;
- current hardware fingerprint recalculation before each proof execution;
- dynamic CUDA driver/runtime and cuBLAS loading without a build-time CUDA SDK;
- GPU selection bound across CUDA UUID and signed NVIDIA telemetry UUID;
- real CUDA VRAM allocation/residency and a cuBLAS SGEMM microbenchmark;
- optional short Ollama inference bound to the exact locally installed model
  digest, challenge nonce, and prompt seed;
- measured wall-clock TTFT and Ollama-reported token throughput;
- automatic signed telemetry capture while VRAM remains resident;
- signed proof response contracts in `burd-protocol`;
- canonical response hash and Ed25519 signature verification against the active
  backend device key;
- backend validation for provider, device, session, fingerprint, GPU UUID,
  backend, model artifact hash, prompt seed, timestamps, CUDA runtime proof,
  VRAM residency proof, GEMM metric, LLM metrics, contention, and telemetry
  window linkage;
- PostgreSQL `proof_challenges` registry, object storage for complete signed
  responses, and challenge audit events.

Not implemented:

- a supervised/background Agent daemon or durable Agent-side retry history;
- a separate Agent-to-backend `running` transition;
- production model artifact distribution or prefetch;
- automatic selection of a model profile by GPU family or model distribution;
  the BN-07 sweep uses one explicitly configured deployment profile and refuses
  to issue recurring challenges while that profile is absent;
- production validation of the CUDA/Ollama executor on every supported NVIDIA
  driver, CUDA runtime, and GPU family;
- scheduler enforcement, paid jobs, billing, Pix, payouts, or complete
  marketplace orchestration.

## Agent Runner

Start the foreground control channel and proof worker with:

```powershell
burd-agent remote-session connect --proofs --telemetry-batch-samples 8
```

`--proofs` implies signed GPU telemetry. The Agent keeps one WebSocket writer so
heartbeat and telemetry control sequences remain ordered. The proof worker polls
the authenticated session endpoint, validates the challenge against the active
identity, session, server expiry, supported proof set, current fingerprint, and
required CUDA backend, then starts the approved executor.

The executor holds a real CUDA allocation while the control loop captures and
submits a fresh one-sample telemetry batch. Only after the backend accepts that
batch does execution continue. The response links the accepted batch hash and is
then canonicalized, hashed, signed with the enrolled Ed25519 key, and submitted.

Local execution errors are logged as `remote_proof_execution_failed`. The Agent
does not fabricate missing metrics or submit a successful response when CUDA,
cuBLAS, NVIDIA telemetry, Ollama, or the exact artifact digest is unavailable.
The challenge remains backend-owned and eventually expires by server time.

### Runtime Requirements

The production executor currently requires:

- an NVIDIA GPU visible through both CUDA and the structured NVIDIA telemetry
  collector;
- NVIDIA driver and CUDA runtime shared libraries;
- cuBLAS when `tensor_gemm_microbenchmark` is required;
- a reachable Ollama API when `llm_short_inference` or LLM thresholds are
  required;
- an installed Ollama model whose reported digest exactly equals
  `model_artifact_hash`.

`OLLAMA_HOST` may override the default `http://127.0.0.1:11434` endpoint. It is
local runtime configuration, not a backend authorization signal.

## API

### `POST /v1/challenges`

Admin endpoint. It issues a proof challenge for an enrolled device and active
remote session. The session must be `online` or `degraded`, and
`required_fingerprint` must match the session fingerprint stored by the backend.
Unknown or duplicate proof requirements are rejected before persistence.

### `GET /v1/sessions/{session_id}/challenges/next`

Device endpoint. It uses the normal remote-session authorization headers and
returns the oldest non-expired `issued` or `acknowledged` challenge. Fetching an
`issued` challenge marks it `acknowledged`.

### `POST /v1/sessions/{session_id}/challenges/{challenge_id}/response`

Device endpoint. It accepts `SignedProofCapabilityResponse`. The signed payload
binds challenge ID, nonce, provider/device/session, profile, current fingerprint,
GPU UUID, CUDA/backend data, exact model artifact hash, prompt seed, metrics,
accepted telemetry window hash, and execution timestamps.

The signature message uses the `burd.proof-capability-response.v1` domain and
`burd-json-c14n-v1` canonicalization.

## State

```text
issued -> acknowledged -> verified | failed | expired
```

`running` remains reserved in the database/API contract. The current Agent does
not call a separate transition endpoint, so a challenge stays `acknowledged`
while local execution is in progress.

## Server Authority

The backend is authoritative for challenge expiry, nonce freshness, active key,
provider/device/session binding, fingerprint/GPU/backend/artifact/prompt checks,
thresholds, telemetry-window linkage, and final status. It never trusts a
provider-sent expiry flag, local capability score, or local eligibility decision.

## Validation Boundary

The PostgreSQL integration harness injects deterministic test-only compute and
telemetry at explicit Agent boundaries. It still runs real enrollment, session,
WebSocket sequencing, telemetry signing, telemetry persistence, challenge
pickup, response canonicalization/signing, object persistence, and backend
verification. This proves the remote protocol without claiming that CI executed
CUDA, cuBLAS, Ollama, or a physical GPU workload.