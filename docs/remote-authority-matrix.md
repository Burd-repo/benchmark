# Remote Authority Matrix

This document defines how the backend treats fields produced by the Provider
Agent. It is the rulebook for avoiding self-attested marketplace truth.

## Authority Classes

- `agent_claimed`: produced by the agent or host. It may be signed and useful,
  but the backend must verify, compare, or downgrade it before using it as
  network truth.
- `agent_signed_evidence`: agent-claimed data bound to an Ed25519 signature,
  report hash, challenge nonce, or hardware fingerprint. Stronger than an
  unsigned claim, but still not backend-attested.
- `backend_attested`: issued, observed, or validated by the backend.
- `backend_derived`: calculated by the backend from accepted evidence,
  backend observations, policy, and server time.
- `never_accepted`: never accepted from the provider as a meaningful external
  fact.

## Identity

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| `provider_id` before enrollment | agent config | `agent_claimed` | Local IDs remain local until registry enrollment. |
| `provider_id` after enrollment | backend registry | `backend_attested` | Backend ID is authoritative. |
| `device_id` | backend registry | `backend_attested` | Separates a human/provider account from a machine. |
| `machine_id` | agent config | `agent_claimed` | Useful continuity signal, not global identity. |
| `public_key` | agent config | `agent_signed_evidence` after nonce proof | Backend must verify private-key possession. |
| private key | local secure storage | `never_accepted` | Must never be transmitted, logged, or stored by backend. |
| contact/location fields | agent config/user input | `agent_claimed` | May inform UX, not region trust. |

## Evidence

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| `report_hash` | agent canonical JSON | `agent_signed_evidence` | Backend verifies canonicalization and signature. |
| `canonicalization_version` | agent report envelope | `agent_signed_evidence` | Backend accepts only supported versions. |
| `signature_valid_locally` | local verifier | `never_accepted` | Backend recalculates signature validity. |
| `hardware_fingerprint` | agent hardware report | `agent_signed_evidence` | Backend compares across evidence, sessions, and challenges. |
| `is_expired` | local freshness helper | `never_accepted` | Backend recalculates with server time. |
| `age_seconds` | local freshness helper | `never_accepted` | Backend recalculates with server time. |
| signed report envelope | agent | `agent_signed_evidence` | Complete envelope goes to object storage; DB stores hash and status. |
| evidence validity status | backend verifier | `backend_derived` | Valid, invalid, expired, revoked, or superseded. |

## Session And Heartbeat

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| `online_locally` | local session/heartbeat | `agent_claimed` | Diagnostic only. |
| remote online/offline | backend session service | `backend_attested` | Derived from authenticated channel and heartbeat policy. |
| heartbeat sequence | agent sends, backend tracks | `backend_attested` after monotonic check | Replays and gaps must be detected. |
| heartbeat timestamp | agent payload | `agent_claimed` | Backend stores receipt time as authority. |
| `last_heartbeat_at` remote | backend receipt | `backend_attested` | Used for availability and reliability. |
| session expiration | backend TTL | `backend_attested` | Provider cannot extend by sending local status. |
| duplicate session state | backend registry | `backend_derived` | Same device/key in multiple sessions is suspicious. |

## Challenge

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| `challenge_id` | backend challenge service | `backend_attested` | Must be globally unique. |
| `nonce` | backend challenge service | `backend_attested` | One-time use. |
| `issued_at`/`expires_at` | backend challenge service | `backend_attested` | Server time only. |
| challenge required profile | backend policy | `backend_attested` | Binds requested work to policy version. |
| response metrics | agent execution | `agent_signed_evidence` | Backend validates thresholds and plausibility. |
| response status | agent execution | `agent_claimed` | Backend calculates final challenge status. |
| challenge verification result | backend verifier | `backend_derived` | Provider-sent verification is ignored. |

## Telemetry And Hardware

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| GPU name/vendor | agent/NVML/system | `agent_claimed` | Useful, but insufficient alone. |
| GPU UUID | agent/NVML | `agent_signed_evidence` | Backend compares over time and across providers. |
| VRAM total/free/used | agent telemetry | `agent_signed_evidence` | Must be checked against hardware class and challenge runs. |
| CUDA/driver versions | agent telemetry | `agent_signed_evidence` | Backend checks consistency and policy support. |
| utilization | agent telemetry | `agent_signed_evidence` | Backend correlates with available/reserved/job states. |
| region | user/agent | `agent_claimed` | Remote probes provide network-region evidence. |
| remote network score | backend probes | `backend_derived` | Never replaced by local benchmark alone. |

## Benchmark Profiles And Results

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| benchmark profile definition | backend/admin policy | `backend_attested` | Provider cannot define its own qualifying workload profile. |
| image digest/model hash/artifact hash | backend profile and signed result | `backend_attested` for profile, `agent_signed_evidence` for result | Backend checks result binding against profile. |
| benchmark result metrics | agent workload execution | `agent_signed_evidence` | Backend verifies signature, ranges, session binding, profile configuration, backend binding, and thresholds. |
| benchmark result hash | agent canonical JSON | `agent_signed_evidence` | Backend recalculates before persistence. |
| benchmark result status | backend verifier | `backend_derived` | `succeeded` or `failed` is decided by backend threshold checks. |
| local AI performance estimate | fit/history/report builder | `agent_claimed` | Diagnostic unless submitted through the BN-10 signed result contract. |
## Secure Provider Runtime

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| runtime readiness check | agent host probe | `agent_claimed` | Diagnostic until bound to backend session, policy, and lease. |
| runtime template allowlist | agent constants in BN-12; backend policy later | `agent_claimed` in BN-12, `backend_attested` after job policy | Provider cannot define templates for paid jobs. |
| runtime image reference | agent CLI input in BN-12; backend job policy later | `agent_claimed` until backend job assignment | Must be digest-pinned before execution planning. |
| image allowlist | agent CLI input in BN-12; backend/admin policy later | `agent_claimed` in BN-12, `backend_attested` for jobs | BN-12 local allowlist avoids accidental unsafe plans, but backend owns production image approval. |
| GPU UUID binding | agent telemetry or CLI input | `agent_signed_evidence` after telemetry/result binding | BN-14 leases bind the exact GPU UUID before assignment. |
| Docker security profile | agent plan | `agent_claimed` | Backend/job runtime must enforce and audit the actual launched container. |
| runtime plan status | agent local calculation | `agent_claimed` | `ready` means locally plannable, not marketplace approval. |
| job execution authorization | none in BN-12 | `backend_attested` | Requires future job, lease, policy, credentials, and audit events. |
| arbitrary shell payload | customer/provider input | `never_accepted` | Jobs must use approved templates, not raw shell commands. |

## Job API And Data Plane

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| job id | backend | `backend_attested` | Generated by control plane when job is created. |
| client job id | admin/customer API input | `backend_recorded` | Optional idempotency reference, not an execution authority. |
| job provider/device/session binding | backend job creation plus session registry | `backend_attested` | BN-13 requires an online or degraded authenticated session. |
| workload eligibility for job admission | backend policy engine | `backend_derived` | Job creation requires `eligible` or `limited`. |
| runtime template for job | backend/admin API input constrained by backend allowlist | `backend_attested` | Provider cannot choose arbitrary templates for assigned jobs. |
| image reference for job | backend/admin API input constrained by digest pinning | `backend_attested` in job metadata | BN-13 requires `@sha256:` before assignment. |
| job artifact manifest | backend/admin API input | `backend_recorded` | BN-13 stores metadata only; byte transfer and checksum enforcement are future data-plane hardening. |
| job-scoped data-plane credential | backend | `backend_attested` | Returned only to the assigned authenticated session; raw credential is not embedded in URLs. |
| provider job progress event | provider session | `agent_claimed` with backend sequencing | Backend binds event to authorized session and rejects duplicate sequence numbers. |
| job terminal result status | provider session, constrained by backend | `agent_claimed` until future verifier/metering | BN-13 accepts `succeeded` or `failed` metadata but does not meter paid usage. |
| job cancellation | backend/admin | `backend_attested` | Cancels only non-terminal jobs. |
| arbitrary shell command | customer/provider input | `never_accepted` | BN-13 accepts approved templates and structured parameters, not shell payloads. |

## Scheduler And Leases

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| scheduler decision | backend scheduler pass | `backend_derived` | Calculated from job state, provider/device/session state, workload eligibility, and active leases. |
| lease id and status | backend scheduler/job control | `backend_attested` | Provider cannot create or extend leases. |
| lease expiry | backend server clock | `backend_derived` | Expired offers are recalculated by the backend, not by provider time. |
| lease job/provider/device/session/GPU binding | backend job plus scheduler validation | `backend_attested` | Assignment is limited to the exact authenticated session that received the offer. |
| provider lease acknowledgement | provider session | `agent_claimed` with backend state transition | Backend accepts it only for the matching assigned job and active lease. |
| double assignment prevention | backend database constraints | `backend_attested` | Active leases are unique per job and per provider/device/GPU. |

## Metering And Usage Ledger

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| usage ledger entry id | backend | `backend_attested` | Generated when a terminal job is metered. |
| receipt hash | backend canonicalization | `backend_attested` | Hash binds the backend-derived receipt payload. |
| reserved GPU seconds | job lease timestamps | `backend_derived` | Calculated from backend lease/job times, not provider claims. |
| actual GPU seconds | job started/completed timestamps | `backend_derived` | Based on backend job lifecycle observations. |
| input/output bytes | job artifact metadata | `backend_recorded` | Metadata-only until byte-level data plane verification exists. |
| retry count | sequenced job events | `backend_derived` | Counts accepted retry event types only. |
| failure classification | backend-constrained result metadata | `backend_derived` | Initial dispute basis, not final payout adjudication. |
| ledger mutation | operator/provider/customer input | `never_accepted` | `usage_ledger_entries` rejects update/delete; corrections require future compensating entries. |
| receipt signature | backend signing key | `backend_attested` when configured | BN-15 stores hash-only receipts until backend receipt signing key management exists. |

## Policy And Marketplace Signals

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| Burd Compute Score | local benchmark report | `agent_signed_evidence` | Evidence input, not marketplace ranking. |
| local reliability score | local uptime history | `agent_claimed` | May seed trust, but backend reliability is authoritative. |
| local trust score | local heuristic | `agent_claimed` | Backend recalculates global trust. |
| local workload eligibility | local policy | `agent_claimed` | Diagnostic only; cannot approve remote marketplace or scheduler use. |
| workload policy definition | backend/admin policy | `backend_attested` | Provider cannot define the policy used for remote eligibility. |
| remote workload eligibility state | backend policy engine | `backend_derived` | Calculated from trust, verification, network, telemetry, signed benchmark results, and policy version. |
| marketplace eligibility | backend policy engine | `backend_derived` | Provider cannot self-approve. |
| pricing/earnings | local estimate | `agent_claimed` | Billing and marketplace pricing are separate future systems. |
