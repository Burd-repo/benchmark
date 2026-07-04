# Provider Console Parity

This document maps the Akash provider-console concepts studied from official
Akash sources into the Burd Benchmark / Burd Agent roadmap. It is intentionally
about functional parity, not visual, brand, text, image, or code copying.

References studied:

- https://github.com/akash-network/console
- https://github.com/akash-network/provider-console-api
- https://github.com/akash-network/provider-console-security
- https://akash.network/docs/api-documentation/rest-api/providers-api/
- https://akash.network/docs/providers/setup-and-installation/provider-console/
- https://akash.network/blog/introducing-akash-provider-console/
- https://akash.network/docs/providers/operations/provider-attributes/

## What Akash Provider Console Appears To Do

Akash's provider experience exposes a provider setup and operations console. The
official documentation describes visual setup, automated installation,
certificate/network configuration, pricing setup, real-time monitoring, provider
status, earnings, deployments/leases, resource usage, activity logs, audit
status, and provider attributes.

The public provider API model exposes provider identity and host information,
health and uptime fields, location, hardware metadata, network speed,
attributes, audited/version status, GPU model metadata, and resource stats for
CPU, GPU, memory, and storage. The security service is separated from the API
service and is responsible for protecting API endpoints with key material,
tokens, expiry durations, CORS, and service host configuration.

## Akash To Burd Mapping

| Akash concept | Burd adaptation |
| --- | --- |
| `owner` | `provider_id` |
| `hostUri` | `host_uri` / local endpoint |
| `email`, `website` | optional provider contact attributes |
| `region`, `country`, `city`, `timezone` | `location` object |
| `uptime1d`, `uptime7d`, `uptime30d` | local uptime ratios from `~/.burd/uptime.json` |
| reliability score | local score derived from heartbeat uptime history |
| network score | local score derived from the latest finite network benchmark |
| `isOnline` | `is_online` from local health/heartbeat |
| `isAudited` | `is_audited` and `audit_status` |
| `attributes` | Burd provider attributes for discovery/readiness |
| `hardwareCpu`, `hardwareCpuArch` | CPU model and architecture from hardware detection |
| `hardwareGpuVendor`, `hardwareGpuModels` | GPU inventory and VRAM from llmfit/Burd hardware report |
| `hardwareMemory`, `hardwareDisk` | RAM and disk signals |
| `networkSpeedDown`, `networkSpeedUp` | measured or placeholder network bandwidth fields |
| `tier` | `burd_tier` from Burd Compute Score |
| active leases | `active_jobs_future` |
| earnings | demonstrative earnings estimate, not real payouts |
| pricing | structured BRL/hour pricing model |
| task logs and action statuses | local `actions.json` and `logs.json` |
| raw data | redacted raw local state and latest report |
| on-chain status | `backend_verification_status_future` |

## What Already Exists In Burd Benchmark

- Rust workspace with `burd-agent`, `burd-api-local`, `burd-bench`,
  `burd-hardware`, `burd-llmfit`, and `burd-protocol`.
- llmfit adapter through `third_party/llmfit/llmfit-core`.
- Hardware detection and Burd-shaped system reports.
- Model fit analyzer and workload recommendations.
- LLM benchmark via Ollama, vLLM, MLX, or auto-detect through llmfit helpers.
- Stability, network, and disk benchmark modules.
- Burd Compute Score, tiers, eligibility, warnings, and demonstrative pricing.
- Full JSON report with skipped sections when benchmarks are not run.
- Basic local identity with a placeholder key.
- Challenge and signed-report data structures at MVP level.
- Local API and a static benchmark UI.
- README, architecture docs, examples, notices, and license handling.

## What Is Missing

- Real local keypair generation and private-key isolation.
- Identity status and key rotation commands.
- Canonical report hashing, report signatures, and local verification.
- Challenge response flow bound to nonce, expiry, report hash, and signature.
- Local health, heartbeat, and uptime history.
- Provider detail aggregation equivalent to a provider-console model.
- Structured pricing and earnings endpoints.
- Verification/audit model with fraud-risk warnings.
- Persistent action/task/log history.
- Raw debug data with redaction.
- API endpoints for provider, uptime, pricing, earnings, actions, logs, raw,
  and verification.
- Provider Console UI tabs for overview, hardware, benchmarks, jobs, earnings,
  uptime, security, logs, and raw data.
- Documentation for security, identity, challenge-response, API, and UI.

## Implementation Plan

1. Stabilize local identity and key management without changing existing command
   behavior.
2. Add report hashing/signing and local verification.
3. Add challenge/response types and CLI commands.
4. Add health, heartbeat, uptime tracking, actions, tasks, and logs.
5. Aggregate local state into `BurdProviderDetails`.
6. Add pricing, earnings, provider verification, and raw-data commands.
7. Expose provider-console data through local API GET endpoints.
8. Replace the MVP benchmark screen with a Burd Provider Console UI that follows
   `SKILL.md`.
9. Add docs and tests around serialization, redaction, signing, uptime,
   provider aggregation, verification, and API shape.

## Mocked For Now

- Active jobs / leases are `0`.
- Pending jobs and completed jobs are `0`.
- Earnings are demonstrative estimates only.
- Backend verification is a future status field.
- Audit status is local/self-verification only.
- Persistent storage support is represented in the model but not provisioned.
- Network up/down bandwidth may be unknown unless a future benchmark measures it.
- Network score is local-only and derived from latency, jitter, loss, and DNS timing.
- Marketplace orchestration, billing, Pix, payouts, and real job execution are
  not implemented.

## Backend Future Dependencies

- Issuing production challenges.
- Verifying signatures and report hashes server-side.
- Provider reputation and fraud scoring.
- Marketplace listing and eligibility decisions.
- Real active jobs, leases, utilization, and earnings.
- Provider payment, billing, Pix, settlement, and tax/reporting flows.
- Remote telemetry, alerting, notifications, and moderation.
- API tokens or mTLS for non-local provider APIs.

## PR 11 Console Integration Notes

Burd now maps Akash-style provider-console monitoring into local-only Burd contracts without claiming parity for remote marketplace behavior. The Workloads tab shows local eligibility and future marketplace status, but never uses `approved`. Trust, capability, network, reliability, and AI performance remain local technical signals until a future backend and remote Proof of Capability exist.