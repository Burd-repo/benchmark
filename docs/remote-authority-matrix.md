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

## Policy And Marketplace Signals

| Field | Local source | Remote authority | Notes |
| --- | --- | --- | --- |
| Burd Compute Score | local benchmark report | `agent_signed_evidence` | Evidence input, not marketplace ranking. |
| local reliability score | local uptime history | `agent_claimed` | May seed trust, but backend reliability is authoritative. |
| local trust score | local heuristic | `agent_claimed` | Backend recalculates global trust. |
| local workload eligibility | local policy | `agent_claimed` | Backend policy decides remote eligibility. |
| marketplace eligibility | backend policy engine | `backend_derived` | Provider cannot self-approve. |
| pricing/earnings | local estimate | `agent_claimed` | Billing and marketplace pricing are separate future systems. |