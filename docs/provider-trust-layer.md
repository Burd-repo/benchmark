# Provider Trust Layer

The Burd Provider Trust Layer is the incremental path from local provider
validation to a future provider network. Its product thesis is:

> Verified AI Compute. Not just listed. Proven.

The local MVP proves technical evidence. It does not register providers in a
backend, list machines in a real marketplace, schedule jobs, create leases, or
implement billing and payouts.

## Concepts

- **Readiness**: whether local identity, signed evidence, challenge evidence,
  history, API authentication, and redaction contracts are valid now.
- **Compute Score**: how much AI compute the machine can deliver. Network
  quality must not erase compute capacity.
- **Network Score**: local quality signal from finite latency, jitter, loss, and DNS samples.
- **Reliability Score**: whether sessions and heartbeats remain stable.
- **Trust Score**: historical confidence from evidence freshness, hardware
  stability, reliability, verification history, and suspicious behavior.
- **Verification Status**: which technical claims have valid signed evidence.
- **Workload Eligibility**: whether a specific workload can use the provider
  locally, diagnostically, or in a future paid marketplace.

Readiness is not marketplace approval. Marketplace eligibility is a separate
policy decision.

## Incremental PR Plan

1. NVIDIA/CUDA marketplace policy and hardware fingerprint.
2. Evidence expiration.
3. Provider session.
4. One-shot heartbeat and utilization snapshot.
5. Local reliability and uptime score.
6. Network score.
7. Local heuristic trust score.
8. Local/mock AI capability spot verification.
9. Workload eligibility.
10. AI performance metric contracts.
11. Provider Console trust UI.
12. Documentation consolidation.

PR 1 through PR 9 are implemented locally. The remaining PRs are future work
and should stay small, deterministic, and independent from a real backend.

## PR 3 Summary

Provider Session adds a local, expirational snapshot that records the moment a
provider tries to become available. It does not create a real backend session
or heartbeat loop.

- `session start` persists the current fingerprint, readiness snapshot,
  signed-report hash, challenge id, evidence summary, and marketplace policy
  snapshot.
- Supported NVIDIA/CUDA hardware starts a marketplace-local session mode.
- Unsupported hardware can still start a local diagnostic session mode, but it
  is not promoted to marketplace eligibility.
- `session status` re-evaluates expiry and invalidation against the latest
  evidence.
- `session stop` marks the local session stopped and offline.
- Registration, provider details, raw data, and readiness can surface a session
  summary when one exists.

See `docs/provider-session.md` for the full contract.

## PR 4 Summary

Heartbeat once adds a local liveness snapshot that is only valid when a local
session is active. It does not create a daemon, a heartbeat loop, a backend
availability signal, or marketplace admission.

- `heartbeat --once --json` reads the current session, validates that it is
  still active, updates `last_heartbeat_at`, increments the local heartbeat
  count, and appends a local uptime/history entry.
- The heartbeat payload includes local hardware and utilization snapshots when
  they are safely available, but unavailable utilization fields remain null.
- A fingerprint change invalidates the session and prevents the heartbeat from
  being counted as online locally.
- Provider details, raw data, registration payloads, and readiness can surface
  the latest heartbeat summary when one exists.

See `docs/heartbeat.md` for the full contract.

## PR 5 Summary

Local reliability and uptime score converts heartbeat history into deterministic
local scoring. It does not contact a backend and does not imply marketplace
availability or admission.

- `uptime --json` includes `uptime_score` and `uptime_level` derived from local
  1d, 7d, and 30d uptime ratios.
- `reliability --json` and `GET /api/v1/reliability` expose a structured
  reliability report with `reliability_score`, status, components, warnings,
  and notes.
- Provider details, raw data, registration payloads, full reports, signed
  reports, and the Provider Console surface the same local reliability signal.
- The score is intentionally local-only and separate from Burd Compute Score,
  readiness, backend availability, and future trust scoring.

## PR 6 Summary

Network score converts the latest finite local network benchmark into a deterministic local signal. It does not start a daemon, bind a port, prove public reachability, or imply marketplace availability.

- `bench network --json` persists the latest finite network sample to local state.
- `network-score --json` and `GET /api/v1/network-score` expose `network_score`, level, status, components, warnings, and notes.
- Provider details, raw data, registration payloads, full reports, signed reports, and the Provider Console surface the same local network signal.
- The score is intentionally separate from backend availability, workload eligibility, public SLA, and future trust scoring.


## PR 7 Summary

Trust score converts local verification, freshness, reliability, network quality, and benchmark history depth into a deterministic local confidence signal. It does not contact a backend and does not imply marketplace admission, payout approval, or workload scheduling.

- `trust-score --json` and `GET /api/v1/trust-score` expose `trust_score`, `level`, `status`, `components`, `warnings`, and `notes`.
- Provider details, raw data, and registration payloads can surface the same local trust summary.
- The score is intentionally heuristic and remains separate from backend approval, marketplace policy, and future workload eligibility.

## PR 8 Summary

Local/mock AI capability spot verification converts fit analysis, runtime readiness, signed evidence, optional live LLM benchmark proof, and local history depth into a deterministic capability signal. It does not create workload eligibility, a scheduler decision, or backend verification.

- `capability-spot --json` and `GET /api/v1/capability-spot` expose `capability_score`, `level`, `status`, `checks`, `evidence`, `warnings`, and `notes`.
- Provider details, raw data, and registration payloads surface the same local/mock capability spot verification report.
- A current signed report with a passing local LLM benchmark is treated as stronger evidence than fit-only capability inference.

## PR 9 Summary

Workload eligibility converts fit recommendations, capability spot verification, trust score, provider verification, reliability, compute score, and marketplace GPU policy into a deterministic local workload decision layer. It does not create a lease, a scheduler assignment, marketplace admission, or a paid job.

- `workload-eligibility --json` and `GET /api/v1/workload-eligibility` expose `local_status`, `marketplace_status_future`, per-workload decisions, confidence levels, reasons, blockers, warnings, and notes.
- Provider details, raw data, and registration payloads surface the same workload eligibility report.
- Local eligibility can be `eligible_locally`, `diagnostic_only`, `not_recommended`, or `blocked`; future marketplace eligibility remains stricter and stays blocked when compute, trust, signed evidence, or marketplace policy are insufficient.

## Future Boundaries

The future backend may receive signed reports, fingerprint evidence, sessions,
heartbeats, spot checks, and registration payloads. A future scheduler should
send jobs only to providers eligible for the requested workload.

Initial orchestration priority:

1. One job to one provider.
2. One job to multiple GPUs on the same machine.
3. Only much later, one job across multiple providers.

Distributed clusters, real leases, jobs, marketplace listing, Pix, billing,
payouts, and heartbeat loops are explicitly outside the local MVP.
