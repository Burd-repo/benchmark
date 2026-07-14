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

## Security Hardening And Attestation

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| security posture hash | agent canonical JSON, recalculated by backend | `agent_signed_evidence` after backend hash check | Backend stores the canonical hash and rejects mismatches. |
| security posture signature | agent Ed25519 key | `agent_signed_evidence` after backend verification | Must verify against an active provider device public key. |
| security posture status | backend policy engine | `backend_derived` | `verified` or `needs_hardening` is decided by backend policy, not by the agent. |
| key storage backend | agent host posture | `agent_claimed` | Backend can require hardware-backed non-exportable posture but does not itself prove TPM/HSM use in BN-20. |
| private key exportability | agent host posture | `agent_claimed` | Useful hardening signal; future attestation/keychain work must strengthen it. |
| release channel and agent version | agent binary/posture | `agent_signed_evidence` after active-key signature | Backend policy decides whether the value is accepted. |
| signed release verification flag | agent local verifier | `agent_claimed` | Backend records and evaluates it; production release signing infrastructure is future. |
| binary hash | agent binary/posture | `agent_signed_evidence` | Stored as hash metadata, not raw binary proof. |
| attestation mode | agent host posture | `agent_claimed` | Accepted mode is policy-gated; remote quote verification is future. |
| attestation evidence hash | agent host posture | `agent_signed_evidence` | Backend stores the hash only; raw quote parsing is not implemented in BN-20. |
| SBOM hash | agent/artifact scanner | `agent_signed_evidence` | Backend can require the hash but does not generate the SBOM in BN-20. |
| vulnerability/dependency scan status | agent/scanner | `agent_claimed` | Backend records and policy-gates status; scanner execution remains future. |
| server receipt time | backend | `backend_attested` | Used for registry ordering and audit. |
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

## Customer Accounts And Reservations

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| human customer user | admin/customer account operation | `backend_attested` | Separate from provider identity and device identity. |
| organization id | backend | `backend_attested` | Owns projects, API keys, credits, reservations, and audit events. |
| project id | backend | `backend_attested` | Workload/customer identity boundary; not a provider identity. |
| customer API key token | returned once by backend | `backend_attested` | Stored only as hash; bearer token authenticates customer reservation APIs. |
| API key scopes | admin API | `backend_attested` | Backend enforces `reservations:read`, `reservations:write`, `usage:read`, `billing:read`, and `billing:write`. |
| project quota | admin API | `backend_attested` | Backend enforces active reservation count, reserved GPU seconds, and TTL. |
| reservation id/status | backend reservation service | `backend_attested` | Customers request reservations; backend decides accepted/cancelled/expired state. |
| reservation listing/provider/GPU binding | backend marketplace listing | `backend_derived` | Reservation binds to a backend-published listing, not a provider claim. |
| reservation idempotency | customer header plus body hash | `backend_attested` | Conflicting replay is rejected. |
| customer usage view | backend reservation and credit tables | `backend_derived` | BN-17 usage is reservation/account balance view, not billing-grade settlement. |
| customer credit ledger | admin/reservation service append | `backend_attested` | Append-only non-settlement credits; corrections require new entries. |
| billing amount | backend settlement over usage, reservation, and price book | `backend_derived` | Provider/customer cannot submit final billing amount. |
| Pix payment confirmation | admin/webhook adapter | `backend_attested` | Payment intent does not affect balance until backend confirmation appends balanced ledger lines. |
| customer financial balance | append-only financial ledger | `backend_derived` | Derived from ledger lines, not mutable account fields. |
| provider payable balance | append-only financial ledger | `backend_derived` | Settlement and payout transactions move provider payable. |
| provider payout amount | admin payout request plus ledger balance | `backend_attested` | Requires KYC/tax status, minimum payout, hold policy, and sufficient payable balance. |
| financial ledger mutation | operator/provider/customer input | `never_accepted` | `financial_ledger_lines` rejects update/delete; corrections require compensating entries. |

## Observability And SRE Signals

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| correlation id | incoming request header or backend generator | `backend_attested` | Diagnostic only; not identity, trust, billing, or audit authority. |
| HTTP request metrics | backend HTTP middleware | `backend_derived` | Aggregate operational state from observed responses. |
| recent request event | backend HTTP middleware | `backend_derived` | Paths are normalized to reduce cardinality and avoid leaking entity IDs. |
| SLO status | backend observability state | `backend_derived` | Indicates operational health, not provider capability or marketplace eligibility. |
| background task error count | backend background tasks | `backend_derived` | Used for SRE triage around session expiration and telemetry retention tasks. |
| log payload mutation | provider/customer input | `never_accepted` | Raw request bodies, bearer tokens, Pix keys, and workload payloads must not become log fields. |
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
| marketplace listing status | backend marketplace sweep | `backend_derived` | Derived from eligibility, trust, verification, benchmark, network, session, and lease state. |
| marketplace current status | backend marketplace sweep and reservation service | `backend_derived` | Uses backend session, active leases, and active customer reservations; provider cannot self-declare available/reserved. |
| marketplace GPU verified flag | backend proof plus benchmark binding | `backend_derived` | Observed GPU UUID alone is never shown as verified marketplace inventory. |
| marketplace VRAM verified flag | backend telemetry bound to verified GPU | `backend_derived` | Self-reported VRAM is not marketplace-verified. |
| marketplace region | regional probes | `backend_derived` | User/provider-declared region is not authoritative. |
| marketplace listing price | admin billing price book | `backend_attested` | BN-18 updates listing price fields from `marketplace_listing_prices`; provider local estimates are not accepted. |
| pricing/earnings | local estimate | `agent_claimed` | Billing and marketplace pricing are separate future systems. |
