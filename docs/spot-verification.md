# Spot Verification

Spot Verification is Burd's local/mock capability check concept. It is the Burd
version of a short proof, but it is scoped to AI capability rather than generic
proof-of-work.

The current implementation is named Capability Spot and is local/mock only.

## Commands And API

```sh
burd-agent capability-spot --json
```

```txt
GET /api/v1/capability-spot
```

## Current Local Contract

The local report contains:

- `capability_score`: score from `0` to `100`;
- `level` and `status`;
- `verification_mode`: currently `local_mock`;
- `checked_at`;
- `summary`;
- optional `top_model`;
- `runnable_models`;
- `recommended_workloads`;
- `components`:
  - `fit_evidence`;
  - `runtime_readiness`;
  - `benchmark_evidence`;
  - `verification_integrity`;
  - `history_support`;
- `checks` with id, label, status, score, max score, and message;
- `evidence` for signed report, challenge, local LLM benchmark, and history;
- `warnings` and `notes`.

## Evidence Strength

Capability Spot is strongest when a current signed report includes a passing
local LLM benchmark. Fit-only inference is useful for diagnostics but is weaker
than measured AI performance.

The local MVP checks:

- model fit evidence;
- runtime/backend readiness;
- current signed evidence;
- optional live LLM benchmark evidence;
- provider verification integrity;
- benchmark history support.

## Difference From Generic PoW

A future remote spot verification should prove that the provider can perform a
specific AI-relevant task now, under a short-lived challenge. It should bind:

- nonce;
- current hardware fingerprint;
- backend/CUDA expectations;
- report hash;
- prompt or task hash;
- measured tokens/s or TTFT threshold when applicable;
- signed response.

## Boundaries

The current capability spot report does not run a remote challenge, does not
contact a backend, does not create marketplace admission, and does not schedule
workloads. It is a local/mock signal consumed by Trust Score, Workload
Eligibility, registration payloads, raw data, provider details, and the Provider
Console.