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

BN-13 implements the first backend-owned job registry and provider pull protocol. BN-14 adds backend-owned scheduler leases that gate assignment before a provider can pull work.

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

## Benchmark Profile API

BN-10 stores backend-owned benchmark profiles and signed benchmark result history. Profiles are defined by admins/control-plane policy. Providers submit signed measurements only for an authenticated remote session.

### `POST /v1/benchmark-profiles`

Admin endpoint that creates or updates one profile version.

Request fields:

- `profile_id`
- `profile_version`
- `workload_type`
- `display_name`
- optional `description`
- `image_digest`
- optional `model_hash`
- optional `artifact_hash`
- `required_backend`
- `min_vram_gb`
- redacted `parameters`
- `warmup_seconds`
- `duration_seconds`
- `sample_count`
- `thresholds`
- optional `status`

### `GET /v1/benchmark-profiles`

Admin endpoint that lists registered profile versions.

### `POST /v1/sessions/{session_id}/benchmark-results`

Device endpoint that submits `SignedBenchmarkResult` through the same authenticated remote-session headers used by telemetry and evidence.

The signed payload contains provider, device, session, run ID, profile ID/version, workload type, backend, hardware fingerprint, GPU UUID, image/model/artifact hashes, parameters, warmup, duration, sample count, timestamps, driver/CUDA versions, metrics, telemetry-window hash, and warnings.

The envelope contains canonical `result_hash`, active `public_key_id`, Ed25519 `signature`, and `canonicalization_version: burd-json-c14n-v1`.

Backend behavior:

- requires an online or degraded session for the provider/device pair;
- recalculates the result hash;
- verifies the signature against the active backend device key;
- binds provider, device, session, fingerprint, profile, workload, backend, image digest, optional model/artifact hash, and profile timing/parameter configuration;
- validates metric ranges, timestamps, sample counts, and redacted JSON fields;
- stores valid results as `succeeded` when thresholds pass or `failed` when thresholds miss;
- deduplicates by `result_hash` and rejects conflicting run IDs.

### `GET /v1/providers/{provider_id}/benchmark-results`

Admin endpoint that lists recent accepted benchmark result records for a provider.

Providers do not submit benchmark profile definitions, final performance status, or marketplace eligibility. The backend owns profile state, verification status, and later policy use.

## Workload Policy API

BN-11 stores backend-owned workload policies and backend-derived eligibility state. Providers do not submit remote eligibility, approval, or marketplace admission.

### `POST /v1/workload-policies`

Admin endpoint that creates or updates one workload policy version.

Request fields:

- `policy_id`
- `policy_version`
- `workload_type`
- `display_name`
- optional `description`
- `requirements`
- optional `status`

Policy requirements can include trust/risk thresholds, reliability and remote network thresholds, verification status and proof freshness, benchmark profile binding, benchmark freshness, backend binding, performance thresholds, minimum VRAM, allowed regions, GPU family, and pricing placeholders for later marketplace policy.

### `GET /v1/workload-policies`

Admin endpoint that lists registered workload policy versions.

### `POST /v1/workload-eligibility/sweep`

Admin endpoint that recalculates provider-device eligibility against active policies.

Backend behavior:

- reads provider/device state, latest remote session, trust state, verification state, regional network state, signed telemetry, and signed benchmark results;
- evaluates active policies only;
- stores `eligible`, `limited`, `ineligible`, `verification_required`, `temporarily_unavailable`, or `blocked`;
- persists reason codes, input scores/statuses, benchmark references, telemetry-derived GPU/VRAM facts, evaluation time, and audit events.

### `GET /v1/providers/{provider_id}/workload-eligibility`

Admin endpoint that lists backend-calculated workload eligibility states for a provider.

BN-11 does not create scheduler assignments, leases, jobs, marketplace listings, billing, Pix, payouts, or background production sweep automation.
## Secure Provider Runtime Contract

BN-12 defines the provider-side runtime plan that BN-13 jobs and BN-14 leases can later bind to backend authority. It is not a job API and does not authorize customer workload execution by itself.

`SecureRuntimePlan` fields include:

- `schema_version: burd-secure-runtime-v1`;
- `policy_version: burd-secure-runtime-policy-v1`;
- `generated_at`;
- `status: ready | verification_required | blocked | unsupported_host`;
- `runtime_engine`, initially `docker+nvidia-container-toolkit`;
- `target_os`, initially ready only on Linux;
- `template_id` from the approved runtime template list;
- optional `image_ref`, which must be digest-pinned with `@sha256:` before execution planning;
- optional `gpu_uuid`, required before lease binding;
- `image_allowlist` entries;
- CPU, memory, PID, and shared-memory limits;
- hardened security profile;
- `docker_args`, emitted only for `ready` local plans;
- structured checks, warnings, and notes.

Initial approved runtime templates:

- `llm_inference`;
- `embeddings`;
- `image_generation`;
- `whisper_transcription`;
- `file_processing`.

Backend behavior reserved for BN-13/BN-14:

- choose the approved template and image digest;
- bind the runtime plan to provider, device, session, GPU UUID, job ID, and lease ID;
- issue job-specific credentials and signed artifact URLs;
- reject arbitrary shell payloads;
- treat provider-generated runtime plans as evidence, not final authority;
- persist runtime/job audit events before execution and after cleanup.

BN-12 by itself does not create jobs, leases, scheduler assignment, data-plane artifact transfer, result upload, metering, billing, Pix, payouts, or marketplace listings. BN-13 adds the first `/v1/jobs` control-plane API and metadata-only data-plane grants.

## Job API And Data Plane

BN-13 creates a backend-authorized job for one provider, one device, one session, and one GPU. BN-14 gates assignment through scheduler-issued leases, while marketplace provider selection remains future work.

### `POST /v1/jobs`

Admin endpoint. Requires `Idempotency-Key`.

Request fields:

- `client_job_id`, optional customer/admin idempotency reference;
- `provider_id`, `device_id`, and `session_id`;
- `workload_type`;
- `template_id`, one of the approved runtime templates;
- `image_ref`, digest-pinned with `@sha256:`;
- `gpu_uuid`;
- `backend`, initially `cuda`;
- structured `parameters`;
- `input_artifacts` and `expected_outputs` manifests;
- optional `timeout_seconds`, `policy_id`, and `policy_version`.

Backend behavior:

- verifies provider/device/session binding;
- requires device status `active`;
- requires session status `online` or `degraded`;
- requires backend workload eligibility `eligible` or `limited`;
- rejects arbitrary shell templates and unpinned images;
- stores `compute_jobs` with status `queued`;
- records an audit event;
- replays the stored response for the same idempotency key and body hash.

### `GET /v1/sessions/{session_id}/jobs/next`

Device-session endpoint. It atomically consumes the oldest non-expired `offered` lease for the authorized provider/device/session, moves the leased job to `assigned`, and returns:

- `job`, the persisted job record;
- `data_plane`, a job-scoped grant;
- `lease`, the scheduler lease record.

The grant contains an opaque credential, server expiry, and scoped artifact paths. URLs do not embed the raw credential. BN-13 does not transfer artifact bytes yet.

### `POST /v1/sessions/{session_id}/jobs/{job_id}/accept`

Device-session endpoint. Moves an assigned job to `accepted` and records optional provider status text.

### `POST /v1/sessions/{session_id}/jobs/{job_id}/events`

Device-session endpoint. Appends a unique sequence number per job. Event types can update status to `provisioning`, `running`, or `uploading`; other events update progress/message metadata only.

### `POST /v1/sessions/{session_id}/jobs/{job_id}/result`

Device-session endpoint. Accepts final `succeeded` or `failed` result metadata, output artifact references, metrics, and optional error fields. Terminal job results cannot be changed.

### Admin Read And Cancel

- `GET /v1/jobs/{job_id}` reads job metadata.
- `GET /v1/providers/{provider_id}/jobs` lists provider jobs with a bounded limit.
- `GET /v1/jobs/{job_id}/leases` lists lease history for one job.
- `GET /v1/providers/{provider_id}/leases` lists provider lease history with a bounded limit.
- `POST /v1/jobs/{job_id}/cancel` moves a non-terminal job to `cancelled` and closes any active lease.

BN-16 adds backend-owned marketplace listing snapshots. The job/data-plane layer still does not implement provider-side execution, object storage signing, byte upload/download enforcement, customer reservations, billing, Pix, payouts, multi-GPU jobs, or multi-provider jobs.

## Scheduler And Leases

### `POST /v1/scheduler/run`

Admin endpoint. Runs one bounded scheduler pass.

Request fields:

- `limit`, optional and capped by backend policy;
- `lease_ttl_seconds`, optional and capped by backend policy;
- `reason`, optional short printable ASCII reason.

Backend behavior:

- expires stale `offered` leases using server time;
- scans queued jobs in creation order;
- requires provider not blocked/quarantined;
- requires active device and `online` or `degraded` session;
- requires workload eligibility of `eligible` or `limited`;
- prevents active duplicate lease for the same job;
- prevents active duplicate lease for the same provider/device/GPU;
- inserts `job_leases` with status `offered` and audit history.

Returns `request_id`, `evaluated`, `offered`, `expired`, `skipped`, and per-job decisions.

### Lease State

```text
offered
-> accepted
-> provisioning
-> active
-> completed | failed | expired
```

Lease timestamps are backend server timestamps. Provider job accept/progress/result calls update lease state in the same job control flow. The provider cannot create, extend, or self-approve leases.

## Metering And Usage Ledger

BN-15 creates backend-derived usage receipts when jobs reach terminal state. The usage ledger is measurement infrastructure only; it is not a billing or payout ledger.

### `POST /v1/jobs/{job_id}/usage-ledger/finalize`

Admin endpoint. Finalizes usage for one terminal job or returns the existing entry when it was already finalized.

Backend behavior:

- requires job status `succeeded`, `failed`, or `cancelled`;
- derives lease and job timing from backend records;
- calculates reserved GPU seconds, actual GPU seconds, billable/non-billable metering basis, idle unbillable seconds, artifact bytes, network transfer bytes, storage bytes, retry count, and failure classification;
- stores canonical `receipt_hash` and `source_hash`;
- appends one `job_usage_finalized` ledger entry per job;
- emits an audit event when a new entry is appended.

### Usage Reads

- `GET /v1/jobs/{job_id}/usage-ledger` lists usage ledger entries for one job.
- `GET /v1/providers/{provider_id}/usage-ledger` lists provider usage ledger entries with a bounded limit.

`usage_ledger_entries` is append-only. Database triggers reject update/delete operations. Later corrections must be modeled as future compensating ledger entries rather than edits.

Receipt signature fields are reserved. Until backend signing key management exists, BN-15 returns `receipt_signature_status = hash_only_backend_signature_not_configured`.

## Marketplace Registry And Listings

BN-16 materializes marketplace listings from backend-owned control-plane state. Providers never submit final marketplace status, verified GPU/VRAM flags, trust score, ranking, price, or availability as trusted truth.

### `POST /v1/marketplace/listings/sweep`

Admin endpoint. Runs one bounded listing registry pass.

Backend behavior:

- reads backend workload eligibility, provider/device status, latest remote session, verification state, trust state, regional network state, signed benchmark result state, and active scheduler leases;
- marks GPU and VRAM as verified only when backend proof state and a succeeded benchmark bind to the observed GPU UUID;
- writes `marketplace_listings` with listing status, current status, region, trust/reliability/network scores, proof freshness, benchmark reference, price placeholder, availability window, active lease count, reason codes, and source hash;
- leaves price fields empty with `price_source = not_configured_bn16`;
- emits audit events for recalculated listing records.

### Listing Reads

- `GET /v1/marketplace/listings` returns `published` and `limited` listings by default, with optional `status`, `workload_type`, and `limit` filters.
- `GET /v1/providers/{provider_id}/marketplace-listings` returns listing registry records for one provider, including non-published statuses for admin inspection.

Listing status can be `published`, `limited`, `verification_required`, `temporarily_unavailable`, or `blocked`. `current_status` reflects operational state such as `available`, `reserved`, `degraded`, `offline`, or `blocked`.

BN-16 does not create customer accounts, reservations, checkout, billing, Pix, payouts, provider-set prices, marketplace ranking, or financial settlement.

## Customer Accounts And Reservations

BN-17 adds customer-side identity and reservation contracts. Customer identity is separate from provider identity and provider device credentials.

### Admin Customer Endpoints

Admin endpoints:

- `POST /v1/customer/users`
- `POST /v1/customer/organizations`
- `GET /v1/customer/organizations/{organization_id}`
- `POST /v1/customer/organizations/{organization_id}/projects`
- `GET /v1/customer/organizations/{organization_id}/audit-events`
- `POST /v1/customer/projects/{project_id}/quotas`
- `POST /v1/customer/projects/{project_id}/api-keys`
- `POST /v1/customer/projects/{project_id}/credits`

Backend behavior:

- stores customer API keys only as hashes;
- returns plaintext customer API key tokens once;
- enforces project quota on reservation count, reserved GPU seconds, and reservation TTL;
- records customer audit events separately from provider identity;
- appends credit entries without mutating prior rows.

### Customer API-Key Endpoints

Customer endpoints use `Authorization: Bearer <customer_api_key>`:

- `GET /v1/customer/projects/{project_id}/reservations`
- `POST /v1/customer/projects/{project_id}/reservations`
- `GET /v1/customer/projects/{project_id}/usage`
- `POST /v1/customer/reservations/{reservation_id}/cancel`

Reservation creation also requires `Idempotency-Key`.

A reservation is accepted only when the API key is active and scoped, the project and organization are active, quota is available, and the target marketplace listing is backend-published with `current_status` of `available` or `degraded`.

BN-17 reservation holds do not create jobs and do not debit billable credits. Credit hold/release entries use zero movement until BN-18 introduces marketplace pricing and financial settlement.

BN-17 does not implement checkout, provider-set pricing, customer job submission, billing, Pix, payouts, invoices, refunds, disputes, or taxes.
## Trust And Antifraud API

BN-09 calculates backend-owned trust and antifraud state from prior remote
signals. The provider does not submit final trust score, risk score, global
reputation, antifraud status, or marketplace eligibility.

### `POST /v1/trust/sweep`

Admin endpoint that runs one bounded global trust and antifraud pass.

Request fields:

- `limit`, optional and capped by backend policy;
- `force`, optional reserved flag for later policy behavior;
- `reason`, optional short printable ASCII reason.

Backend behavior:

- reads provider, device, latest session, heartbeat, telemetry, evidence,
  challenge, verification, and remote network state;
- recalculates `trust_score`, `risk_score`, backend reliability, status, and
  reason codes;
- upserts `provider_trust_states` by `(provider_id, device_id)`;
- records active antifraud events for backend-observed suspicious conditions;
- emits an audit event for each recalculated trust state.

Returns `request_id`, `evaluated`, and an `updated` list with provider, device,
status, trust score, risk score, and reason codes.

### `GET /v1/providers/{provider_id}/trust-states`

Admin endpoint that lists backend-calculated trust states for provider devices.
Rows include status, policy version, trust score, risk score, backend
reliability score, verification status, remote network score, evidence and
challenge counts, latest session status, latest GPU UUID, hardware fingerprint,
reason codes, and timestamps.

### `GET /v1/providers/{provider_id}/antifraud-events`

Admin endpoint that lists recent antifraud events for a provider. The optional
`limit` query parameter is clamped to `1..200`.

Events include type, severity, status, reason, redacted metadata, first/last
seen timestamps, and occurrence count.

BN-09 does not automatically quarantine or block providers, feed scheduler
assignments, rank marketplace listings, run jobs, meter usage, bill customers,
or pay providers.

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
