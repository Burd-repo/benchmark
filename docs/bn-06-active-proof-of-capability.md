# BN-06 - Active Proof Of Capability Protocol

BN-06 adds the first remote Proof of Capability surface to the Burd Control Plane.
It does not add recurring verification, scheduler decisions, jobs, marketplace, or billing.

## Scope

Implemented:

- backend-issued proof challenges for an online or degraded remote session;
- backend-attested `challenge_id`, nonce, issue time, expiry, required fingerprint, optional GPU UUID, backend, artifact hash, prompt seed, thresholds, and required proof names;
- session-authenticated challenge pickup by the enrolled device;
- signed proof response contracts in `burd-protocol`;
- canonical response hash and Ed25519 signature verification against the active backend device key;
- backend validation for provider, device, session, fingerprint, GPU UUID, backend, model artifact hash, prompt seed, timestamps, CUDA runtime proof, VRAM residency proof, GEMM metric, LLM short inference metrics, contention flag, and telemetry window hash;
- PostgreSQL `proof_challenges` registry with status, response hash, public key ID, response object key, and verification JSON;
- filesystem-backed object storage for full signed proof response envelopes;
- audit events for issued, acknowledged, verified, failed, and expired proof challenges.

Not implemented:

- agent-side execution of the CUDA/VRAM/GEMM/LLM proof workload;
- risk-based recurring challenge scheduling;
- trust score or antifraud score recalculation;
- scheduler enforcement, leases, jobs, billing, Pix, payouts, or marketplace listings.

## API

### `POST /v1/challenges`

Admin endpoint. Issues a proof challenge for an already enrolled device and active remote session.
The session must be `online` or `degraded`, and `required_fingerprint` must match the session fingerprint stored by the backend.

Request fields:

- `provider_id`
- `device_id`
- `session_id`
- `profile_version`
- `required_fingerprint`
- `required_gpu_uuid`, optional
- `required_backend`, initially `cuda`
- `model_artifact_hash`
- `prompt_seed`
- `required_proofs`, optional; defaults to the initial BN-06 proof set
- `min_tokens_per_second`
- `max_ttft_ms`
- `expires_in_seconds`, optional and capped by `BURD_CONTROL_PROOF_CHALLENGE_TTL_SECONDS`

### `GET /v1/sessions/{session_id}/challenges/next`

Device endpoint. Uses the normal remote-session headers:

- `Authorization: Bearer <device credential>`
- `X-Burd-Session-Token`
- `X-Burd-Device-Id`

Returns the oldest non-expired `issued` or `acknowledged` challenge for the session and marks an `issued` challenge as `acknowledged`.

### `POST /v1/sessions/{session_id}/challenges/{challenge_id}/response`

Device endpoint. Submits `SignedProofCapabilityResponse`.

The signed payload binds:

- challenge ID and nonce;
- provider, device, and session IDs;
- profile version;
- hardware fingerprint and GPU UUID;
- backend and CUDA proof data;
- model artifact hash and prompt seed;
- metrics and telemetry window hash;
- execution start and completion timestamps.

The signature message uses the `burd.proof-capability-response.v1` domain and the `burd-json-c14n-v1` canonicalization version.

## State

```text
issued -> acknowledged -> verified | failed | expired
```

`running` is reserved in the database/API contract for the agent-side execution phase, but BN-06 does not require a separate running endpoint.

## Server Authority

The backend is authoritative for:

- challenge expiry;
- nonce freshness;
- key binding;
- provider/device/session binding;
- fingerprint/GPU/backend/artifact/prompt checks;
- threshold checks;
- final `verified`, `failed`, or `expired` status.

The backend does not trust any provider-sent expiry flag, local capability score, or local eligibility decision.
