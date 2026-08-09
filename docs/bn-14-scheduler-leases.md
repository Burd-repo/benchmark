# BN-14 - Scheduler And Leases

BN-14 adds the first backend-owned scheduler pass and lease registry. It consumes the BN-13 job table and BN-11 workload eligibility state, then offers short-lived leases for queued jobs that are already bound to one provider, one device, one active session, and one GPU.

BN-14 does not create marketplace demand matching, price optimization, paid execution, metering, billing, Pix, payouts, arbitrary shell execution, multi-GPU jobs, or multi-provider jobs. It is the control-plane reservation layer that BN-15 metering can consume.

## Backend Scope

BN-14 adds:

- `job_leases`, a backend-attested lease table linked to `compute_jobs`;
- `burd-protocol` lease records, scheduler request/response contracts, and lease list responses;
- `POST /v1/scheduler/run` for an admin-triggered bounded scheduler pass;
- `GET /v1/jobs/{job_id}/leases` and `GET /v1/providers/{provider_id}/leases` for lease history;
- provider `jobs/next` behavior that requires both a non-expired offered lease and current Runtime Admission before assignment;
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
- no active lease exists for the same provider/device/GPU;
- a transaction-local `RuntimeAdmissionDecision` for the exact provider/device/GPU is `admitted`.

Denied Runtime Admission never creates a lease. It returns a scheduler decision with
`decision=skipped`, no `lease_id`, and stable admission reason codes. Offered-lease audit metadata
records the verification ID, runtime verification fingerprint, runtime observation hash and the
single authoritative evaluation time used by that scheduler pass.

## Batching And Fairness

The request `limit` caps offered leases. The scheduler scans candidates in batches of 50 and uses a
separate bounded evaluation budget of 50-800 candidates per pass. It records
`scheduler_last_evaluated_at` for every evaluated job and orders later passes by the oldest
evaluation, then creation time and job ID. A prefix of denied jobs therefore rotates behind
untouched work instead of consuming the same SQL `LIMIT` forever.

Candidate rows remain protected by `FOR UPDATE OF j SKIP LOCKED`. A transaction-scoped advisory
lock for the normalized provider/device/GPU tuple is acquired before lease creation, followed by a
fresh active-lease check. The existing partial unique indexes remain the final database invariant.

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

`GET /v1/sessions/{session_id}/jobs/next` no longer scans arbitrary queued jobs directly. Each poll locks a bounded set of up to 16 non-expired `offered` leases plus their queued jobs with `FOR UPDATE ... SKIP LOCKED`, ordered oldest first. For each offer it re-evaluates Runtime Admission for the exact authenticated provider/device and leased GPU using a fresh server `now` in the same transaction.

When admission was lost, the Control Plane emits no credential or execution bundle, marks the lease `expired` with `failure_reason=runtime_admission_lost_before_assignment`, keeps the job `queued`, clears credential fields defensively, records `lease.assignment_withheld` with current reason codes, and continues to the next locked offer. One stale GPU therefore cannot block another valid GPU in the same poll.

Only a currently admitted offer moves the job to `assigned` and returns:

- the job record;
- the job-scoped data-plane grant;
- the lease record.

The assignment audit records the current verification ID, runtime verification fingerprint, observation hash and evaluation time. A newer valid proof may replace the proof used when the lease was offered. The plaintext job credential is returned once in the data-plane grant; only its hash and expiry are persisted.

This prevents a provider from pulling work that the scheduler did not reserve for that exact session or whose runtime authority changed after scheduling.

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
