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
- **Network Score**: which remote workload profiles the connection can support.
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

PR 1 through PR 4 are implemented locally. The remaining PRs are future work
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
