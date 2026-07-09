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

BN-03 client message types:

- `heartbeat`

Reserved client message types for BN-04 and later:

- `telemetry_batch`
- `challenge_ack`
- `challenge_started`
- `challenge_submitted`

BN-03 server message types:

- `session_ready`
- `session_revoked`
- `heartbeat_ack`
- `error`

Reserved server message types for BN-04 and later:

- `challenge_issued`
- `policy_update`

### `POST /v1/sessions/{session_id}/heartbeats`

HTTP fallback for one heartbeat when the stream is unavailable. It follows the
same sequence and server-time rules as the control channel.

## Evidence API

### `POST /v1/evidence-records`

Submits signed evidence for backend verification and storage.

Request fields:

- `provider_id`
- `device_id`
- `session_id`
- `evidence_type`
- `canonicalization_version`
- `hash`
- `signed_envelope`
- `signature`
- `public_key_id`
- `hardware_fingerprint`
- `agent_claimed_issued_at`

Backend behavior:

- recalculates canonical hash when possible;
- verifies signature with the active public key;
- stores full envelope in object storage;
- stores hash, object pointer, status, timestamps, and verification result in
  PostgreSQL;
- recalculates freshness from server policy and server time;
- emits audit events for accepted, rejected, expired, duplicate, and revoked
  evidence.

### `GET /v1/evidence-records/{evidence_id}`

Returns metadata and backend verification state. It does not need to return the
full object-storage envelope by default.

## Challenge API

### `POST /v1/challenges`

Issues a backend challenge for a provider/device/session.

Request fields:

- `provider_id`
- `device_id`
- `session_id`
- `profile_version`
- `required_fingerprint`
- `required_gpu_uuid`
- `required_backend`
- `model_artifact_hash`
- `prompt_seed`
- `min_tokens_per_second`
- `max_ttft_ms`

Returns the backend-attested challenge:

- `challenge_id`
- `nonce`
- `profile_version`
- `issued_at`
- `expires_at`
- required proof fields

### `POST /v1/challenges/{challenge_id}/responses`

Submits a signed challenge response.

Request fields:

- `challenge_id`
- `nonce`
- `provider_id`
- `device_id`
- `session_id`
- `response_hash`
- `signature`
- `public_key_id`
- `hardware_fingerprint`
- `gpu_uuid`
- `driver`
- `cuda`
- `metrics`
- `telemetry_window_hash`
- `started_at`
- `completed_at`

Backend behavior:

- rejects unknown, expired, revoked, or reused nonces;
- verifies signature and response hash;
- checks required fingerprint, GPU UUID, backend, artifact hash, and metrics;
- binds response to accepted telemetry window when available;
- sets challenge state to `verified`, `failed`, or `expired`.

## Telemetry API

Telemetry is normally sent through the control channel as `telemetry_batch`.
BN-01 may also expose an HTTP fallback:

### `POST /v1/sessions/{session_id}/telemetry-batches`

Request fields:

- `provider_id`
- `device_id`
- `session_id`
- `sequence_start`
- `sequence_end`
- `hardware_fingerprint`
- `samples`
- `batch_hash`
- `signature`

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