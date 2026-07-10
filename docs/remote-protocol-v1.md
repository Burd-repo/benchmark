# Remote Protocol v1

This document freezes the BN-00 remote protocol shape for the first Burd
Control Plane. It is a design contract for BN-01 and later implementation.

The backend path prefix is `/v1`. The existing local agent API under
`/api/v1` remains local-only and is not the Burd Network control-plane API.

## Protocol Rules

- JSON request and response bodies use UTF-8.
- Timestamps use RFC3339 UTC.
- IDs are opaque strings. UUID-backed IDs are acceptable, but clients must not
  parse semantics from IDs.
- Ed25519 is the first signing algorithm.
- Canonical JSON uses `burd-json-c14n-v1` unless a later version is explicitly
  negotiated.
- Mutating HTTP requests use `Idempotency-Key`.
- Successful mutating responses include a server `request_id` and an audit
  event reference when one is emitted.
- Provider-sent freshness flags, scores, eligibility flags, and local online
  flags are never authoritative.
- Backend receipt time is authoritative over provider wall-clock time.

## Authentication

- Human/admin endpoints use the backend account credential model selected in
  BN-01. Provider devices cannot call those endpoints with device credentials.
- Enrollment uses a short-lived one-time enrollment token plus an Ed25519
  nonce proof. The token alone is not enough to enroll a device.
- Session and control-channel requests use a short-lived device credential
  issued after enrollment proof.
- Evidence, challenge responses, and telemetry batches are signed by the
  enrolled provider key or a rotated active key.
- Revoked keys, revoked devices, expired credentials, expired enrollment
  tokens, and expired challenges fail closed.

## Error Envelope

All non-2xx responses use:

```json
{
  "error": {
    "code": "invalid_request",
    "message": "human-readable summary",
    "request_id": "req_...",
    "retry_after_seconds": null,
    "details": {}
  }
}
```

Stable error codes:

- `invalid_request`
- `unauthorized`
- `forbidden`
- `not_found`
- `conflict`
- `idempotency_conflict`
- `rate_limited`
- `expired`
- `revoked`
- `signature_invalid`
- `nonce_reused`
- `policy_blocked`
- `internal`

## Provider State Machine

```text
unregistered
-> enrolled
-> pending_verification
-> verified
-> available
-> reserved
-> hired
-> degraded
-> quarantined
-> blocked
-> offline
```

State authority:

- the agent can request transitions by connecting, disconnecting, submitting
  evidence, or accepting work;
- the backend decides the persisted state;
- scheduler, jobs, and billing do not exist in BN-01, but `reserved` and
  `hired` remain reserved state names for later.

## Remote Session State Machine

```text
none
-> pending_connection
-> online
-> degraded | offline
-> online
-> expired | revoked
```

Rules:

- only one active session is allowed for a `(provider_id, device_id)` pair;
- every heartbeat/control message carries a monotonic sequence number;
- missed heartbeat policy is evaluated by server receipt time;
- reconnect uses backoff and may resume only when session TTL and credential
  validity allow it;
- remote revocation wins over local session state.

## Challenge State Machine

```text
issued
-> acknowledged
-> running
-> submitted
-> verified | failed | expired
```

Rules:

- `challenge_id`, `nonce`, `issued_at`, and `expires_at` are backend-attested;
- nonce is one-time use;
- challenge expiry is calculated by the backend;
- response signatures bind nonce, challenge ID, report hash, fingerprint, and
  required proof fields;
- backend verification status is final.

## Job State Machine

Jobs are not implemented in BN-01, but their names are reserved so provider
state and API wording do not conflict later.

```text
queued
-> assigned
-> accepted
-> provisioning
-> running
-> uploading
-> succeeded | failed | cancelled
```

## Enrollment API

### `POST /v1/providers`

Creates a backend provider record. This is a human/admin/customer-account
operation in BN-01 and does not enroll a device by itself.

Returns:

- `provider_id`
- `status: enrolled | unregistered`
- `created_at`

### `POST /v1/providers/{provider_id}/enrollment-tokens`

Issues a short-lived one-time enrollment token.

Returns:

- `enrollment_token`
- `expires_at`
- `max_uses: 1`

### `POST /v1/enrollments`

Starts device enrollment.

Request fields:

- `enrollment_token`
- `public_key`
- `key_algorithm`
- `registration_payload`
- `hardware_fingerprint`
- `agent_version`
- `benchmark_version`

Backend behavior:

- validates token expiry and one-time use;
- stores the submitted registration payload as untrusted evidence;
- issues a nonce for key-possession proof.

Returns:

- `enrollment_id`
- `provider_id`
- `nonce`
- `expires_at`

### `POST /v1/enrollments/{enrollment_id}/proof`

Completes device enrollment by proving possession of the private key.

Request fields:

- `nonce`
- `signature`
- `public_key`
- `hardware_fingerprint`

Returns:

- `provider_id`
- `device_id`
- `credential`
- `credential_expires_at`
- `status: pending_verification`

The private key is never transmitted. The signed canonical proof uses the
`burd.enrollment-proof.v1` domain and binds `enrollment_id`, backend
`provider_id`, `machine_id`, nonce, public key, hardware fingerprint, and
server-issued expiry.

### `POST /v1/devices/{device_id}/credentials`

Rotates the short-lived device bearer credential. The current credential is
required; successful rotation revokes it in the same transaction and returns
the replacement once.

### `POST /v1/devices/{device_id}/key-rotations`

Starts coordinated key rotation under the current device credential. The
backend stores the proposed public key and returns a nonce and expiry.

### `POST /v1/devices/{device_id}/key-rotations/{rotation_id}/proof`

Verifies canonical `burd.key-rotation-proof.v1` claims signed by the proposed
new key. Success activates the new key and revokes the previous key atomically.

### `GET /v1/providers/{provider_id}/devices`

Returns backend device state and active public-key IDs without credentials or
raw key material.

### `POST /v1/devices/{device_id}/revoke`

An administrative action that revokes the device, identity, active key,
credentials, and pending rotations.

## Session And Control Channel API

### `POST /v1/sessions`

Starts a remote session for an enrolled device.

Request fields:

- `provider_id`
- `device_id`
- `hardware_fingerprint`
- `latest_report_hash`
- `latest_challenge_id`
- `agent_version`
- `capabilities`

Returns:

- `session_id`
- `status`
- `expires_at`
- `heartbeat_interval_seconds`
- `missed_heartbeat_limit`
- `sequence_start`
- `telemetry_sequence_start`
- `control_url`
- `resume_token`

### `GET /v1/sessions/{session_id}/control`

Upgrades to the BN-03 WebSocket control channel. Device authentication and the
session resume token are sent in headers, never in the URL:

- every client message includes `session_id`, `device_id`, `sequence`, `sent_at`,
  `type`, and `payload`;
- every server message includes `request_id`, `sequence_ack`, `type`, and
  `payload`;
- sequence gaps, duplicates, and stale sessions are audit events.

BN-04 client message types:

- `heartbeat`
- `telemetry_batch`

Reserved client message types for BN-06 and later:

- `challenge_ack`
- `challenge_started`
- `challenge_submitted`

BN-04 server message types:

- `session_ready`
- `session_revoked`
- `heartbeat_ack`
- `telemetry_ack`
- `telemetry_rejected`
- `error`

Reserved server message types for BN-06 and later:

- `challenge_issued`
- `policy_update`

### `POST /v1/sessions/{session_id}/heartbeats`

HTTP fallback for one heartbeat when the stream is unavailable. It follows the
same sequence and server-time rules as the control channel.

## Evidence API

### `POST /v1/sessions/{session_id}/evidence-records`

Submits signed evidence for backend verification and registry storage. The
request uses the authenticated remote-session headers:

- `Authorization: Bearer <device credential>`;
- `X-Burd-Session-Token`;
- `X-Burd-Device-Id`.

Initial BN-05 request fields:

- `evidence_type`, defaulting to `signed_report`;
- `session_id`, optional duplicate binding to the path session;
- `subject_id`, optional future challenge/capability/benchmark subject;
- `metadata`, optional non-secret JSON;
- `signed_report`, the complete `SignedReport` envelope.

Backend behavior:

- authenticates the device credential and session resume token;
- requires a nonterminal remote session;
- recalculates the canonical report hash;
- recalculates the canonical evidence envelope hash;
- verifies Ed25519 with the active backend device key;
- binds provider, device, machine ID, session, active key, and hardware
  fingerprint;
- stores the complete signed envelope in object storage;
- stores hash, object pointer, status, timestamps, and verification result in
  PostgreSQL;
- deduplicates by `evidence_hash`;
- emits audit events for accepted and rejected evidence.

The backend never trusts `is_expired` from the agent. It recalculates evidence
freshness from `signed_at`, server policy, and server time. Expired but
otherwise valid evidence can be stored with `status=expired`; invalid hash,
signature, key binding, provider binding, device binding, or fingerprint is
rejected.

### `GET /v1/providers/{provider_id}/evidence-records`

Admin endpoint that lists evidence metadata and backend verification state for a
provider. It does not return full object-storage envelopes by default.

### `GET /v1/evidence-records/{evidence_id}`

Admin endpoint that returns one evidence metadata record and backend
verification state.

### `POST /v1/evidence-records/{evidence_id}/revoke`

Admin endpoint that marks an evidence record as revoked. Revocation updates
registry metadata and audit history; it does not delete the stored envelope.

## Challenge API

BN-06 implements active proof-of-capability challenges for an already enrolled
provider device with an active remote session. BN-07 adds recurring/risk-based
backend verification state around those challenges.

### `POST /v1/challenges`

Admin endpoint that issues a backend-attested proof challenge. The target
session must be `online` or `degraded`, and `required_fingerprint` must match
the backend session fingerprint.

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
- `required_proofs`, optional
- `min_tokens_per_second`
- `max_ttft_ms`
- `expires_in_seconds`, optional and capped by backend config

Returns `ProofCapabilityChallenge` with backend-generated `challenge_id`,
`nonce`, `issued_at`, `expires_at`, and the required proof fields.

### `GET /v1/sessions/{session_id}/challenges/next`

Device endpoint authenticated with the remote-session headers. It returns the
oldest non-expired `issued` or `acknowledged` challenge for the session and
marks newly delivered challenges as `acknowledged`.

### `POST /v1/sessions/{session_id}/challenges/{challenge_id}/response`

Device endpoint that submits `SignedProofCapabilityResponse`.

The signed payload contains:

- `schema_version: burd-proof-capability-response-v1`
- `challenge_id`
- `nonce`
- `provider_id`
- `device_id`
- `session_id`
- `profile_version`
- `hardware_fingerprint`
- `gpu_uuid`
- `backend`
- `model_artifact_hash`
- `prompt_seed`
- `driver_version`
- optional CUDA driver/runtime versions
- `metrics`
- optional `telemetry_window_hash`
- `started_at`
- `completed_at`

The envelope contains:

- `payload`
- `response_hash`, the canonical hash of `payload`
- `public_key_id`
- `signature`
- `canonicalization_version: burd-json-c14n-v1`

The signature message uses domain `burd.proof-capability-response.v1` and binds
response hash, challenge ID, nonce, provider, device, session, profile,
fingerprint, GPU UUID, backend, artifact hash, prompt seed, and public key ID.

Backend behavior:

- rejects unknown, terminal, or expired challenges;
- recalculates the response hash;
- verifies Ed25519 with the active backend device key;
- binds response to provider, device, session, fingerprint, GPU UUID, backend,
  artifact hash, and prompt seed;
- checks execution timestamps against challenge issue/expiry and server clock;
- evaluates initial CUDA runtime, VRAM residency, GEMM, short LLM inference,
  contention, and telemetry-window proof fields;
- stores the complete signed response in object storage;
- stores response metadata and verification JSON in PostgreSQL;
- sets challenge state to `verified`, `failed`, or `expired`.

## Verification Policy API

BN-07 tracks backend-owned verification state per `(provider_id, device_id)` and uses BN-06 challenges as the active proof mechanism.

States:

```text
new_provider -> verification_due -> verification_running -> verified
verified -> verification_due -> verification_running -> verified | suspect
suspect -> verification_due | quarantined | blocked
```

`quarantined` and `blocked` are reserved for later policy/admin action. Sweeps do not issue new challenges for those states.

### `POST /v1/verification/sweep`

Admin endpoint that runs one bounded recurring/risk verification pass.

Request fields:

- `limit`, optional and capped by backend config;
- `force`, optional;
- `reason`, optional short printable ASCII reason.

Backend behavior:

- expires stale proof challenges using server time;
- converts expired running verification states into failed verification state;
- evaluates `online` and `degraded` sessions only;
- skips blocked/quarantined providers, inactive devices, sessions without backend hardware fingerprint, and sessions that already have an active proof challenge;
- issues BN-06 challenges for new, due, suspect, forced, or stale-running verification states;
- binds issued challenges to the backend session fingerprint and latest accepted GPU telemetry UUID when available.

Returns `request_id`, `evaluated`, and an `issued` list with provider, device, session, challenge ID, and reason.

### `GET /v1/providers/{provider_id}/verification-states`

Admin endpoint that lists backend verification state for provider devices.

Rows include status, policy version, reason, risk score, success/failure counts, retry budget, last challenge IDs, last verified/failed timestamps, next due timestamp, and reserved quarantine/block timestamps.

BN-07 does not publish final trust ranking. It only persists the recurring verification state that later trust, policy, scheduler, and marketplace code can consume.

## Network Probe API

BN-08 records trusted regional observations for a provider device's existing
remote session. The provider does not submit `remote_network_score` or final
reachability. Probes observe the authenticated outbound control path and future
data-plane paths without requiring an inbound public port on the provider.

### `POST /v1/network-probes/observations`

Admin/probe endpoint that stores one trusted observation and recalculates the
provider-device network state.

Request fields:

- `provider_id`
- `device_id`
- `session_id`
- `probe_id`
- `probe_region`
- `observed_at`
- `sample_count`
- optional `control_rtt_ms`, `jitter_ms`, and `packet_loss_percent`
- optional `reconnect_count`
- optional `upload_mbps`, `download_mbps`, and `artifact_throughput_mbps`
- optional `stability_score`
- optional `approximate_region`
- optional `path_consistency`
- optional redacted `metadata`

Backend behavior:

- requires the provider, device, and remote session binding to match;
- rejects blocked/quarantined providers and inactive devices;
- accepts observations for `online`, `degraded`, or `offline` sessions;
- validates metric ranges, timestamp shape, and redacted metadata;
- deduplicates by `(session_id, probe_id, observed_at)`;
- calculates `remote_network_score`, regional reachability, and effective score
  server-side;
- emits an audit event for newly accepted observations.

### `GET /v1/providers/{provider_id}/network-probes`

Admin endpoint that lists recent trusted network probe observations for a
provider.

### `GET /v1/providers/{provider_id}/network-state`

Admin endpoint that returns backend-calculated network state rows for provider
devices. Rows include nullable `local_network_score`, `remote_network_score`,
`regional_reachability`, `effective_network_score`, sample count, last observed
time, and update time.

BN-08 does not deploy the production regional probe fleet or consume these
scores in scheduler, marketplace, billing, or global trust ranking.
## Telemetry API

Telemetry is normally sent through the control channel as `telemetry_batch`.
BN-04 also exposes an authenticated HTTP fallback:

### `POST /v1/sessions/{session_id}/telemetry-batches`

Request fields:

- `session_id`
- `device_id`
- `sequence`
- `sent_at`
- `message_type: telemetry_batch`
- `payload`, containing the complete `SignedTelemetryBatch`

The signed payload contains provider, device, and session IDs; control and
sample sequence ranges; hardware fingerprint; collector and collection window;
samples; canonical batch hash; active public-key ID; canonicalization version;
and Ed25519 signature.

Samples may include GPU UUID, PCI IDs, compute capability, driver, CUDA
runtime/driver, VRAM total/used/free, GPU utilization, memory utilization,
temperature, power, clocks, throttling, ECC, and process/container association
when available.

## Audit Events

Every state-changing backend decision emits an audit event with:

- `audit_event_id`
- `request_id`
- `actor_type`
- `actor_id`
- `entity_type`
- `entity_id`
- `event_type`
- `occurred_at`
- `idempotency_key`
- `summary`
- optional `metadata`

Audit records are append-only.
