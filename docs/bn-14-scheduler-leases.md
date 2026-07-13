# BN-14 - Scheduler And Leases

BN-14 adds the first backend-owned scheduler pass and lease registry. It consumes the BN-13 job table and BN-11 workload eligibility state, then offers short-lived leases for queued jobs that are already bound to one provider, one device, one active session, and one GPU.

BN-14 does not create marketplace demand matching, price optimization, paid execution, metering, billing, Pix, payouts, arbitrary shell execution, multi-GPU jobs, or multi-provider jobs. It is the control-plane reservation layer that BN-15 metering can consume.

## Backend Scope

BN-14 adds:

- `job_leases`, a backend-attested lease table linked to `compute_jobs`;
- `burd-protocol` lease records, scheduler request/response contracts, and lease list responses;
- `POST /v1/scheduler/run` for an admin-triggered bounded scheduler pass;
- `GET /v1/jobs/{job_id}/leases` and `GET /v1/providers/{provider_id}/leases` for lease history;
- provider `jobs/next` behavior that requires a non-expired offered lease before assignment;
- lease lifecycle updates when the provider accepts, provisions, runs, completes, fails, or the backend cancels a job;
- audit events for offered leases.

## Scheduler Inputs

A queued job can receive a lease only when:

- the job is still `queued`;
- provider status is not `blocked` or `quarantined`;
- device status is `active`;
- session status is `online` or `degraded`;
- backend workload eligibility for the job workload is `eligible` or `limited`;
- requested policy ID/version, when present, match the eligibility record;
- no active lease exists for the same job;
- no active lease exists for the same provider/device/GPU.

## Lease State Machine

```text
offered
-> accepted
-> provisioning
-> active
-> completed | failed | expired
```

`offered` leases are short-lived and expire by server time. A later scheduler pass marks stale offers as `expired` and returns the still-queued job to the pool when the job had only been assigned through that stale offer.

Job cancellation closes any active lease as failed with a cancellation reason. The lease remains in history for audit and later dispute/metering flows.

## Assignment Rules

`GET /v1/sessions/{session_id}/jobs/next` no longer scans arbitrary queued jobs directly. It consumes the oldest non-expired `offered` lease for the authenticated provider/device/session, marks the job as `assigned`, and returns:

- the job record;
- the job-scoped data-plane grant;
- the lease record.

This prevents a provider from pulling work that the scheduler did not reserve for that exact session.

## Deferred

BN-14 intentionally leaves out:

- autonomous/background scheduler daemon cadence;
- marketplace provider search, customer reservations, and pricing optimization;
- scheduler selection across unbound marketplace supply;
- paid container execution on the provider host;
- byte-level artifact transfer and object-storage signed URLs;
- usage metering, job receipts, billing, Pix, payouts, refunds, or disputes;
- multi-GPU and multi-provider placement;
- Kubernetes or distributed workload orchestration.