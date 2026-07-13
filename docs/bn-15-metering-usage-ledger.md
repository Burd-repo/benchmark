# BN-15 - Metering And Usage Ledger

BN-15 adds the first backend-owned usage ledger for compute jobs. It derives usage from persisted jobs, scheduler leases, job events, artifact metadata, and server timestamps, then appends an immutable usage receipt when a job reaches a terminal state.

BN-15 does not charge customers, calculate invoices, execute Pix, release payouts, settle disputes, or transfer artifact bytes. It creates the reproducible measurement layer that BN-18 billing and payout logic can later consume.

## Backend Scope

BN-15 adds:

- `usage_ledger_entries`, an append-only PostgreSQL ledger table;
- `burd-protocol` usage receipt, usage ledger entry, finalize response, and list response contracts;
- automatic usage finalization on job `succeeded`, `failed`, and `cancelled` transitions;
- `POST /v1/jobs/{job_id}/usage-ledger/finalize` for admin replay/backfill of terminal jobs;
- `GET /v1/jobs/{job_id}/usage-ledger` and `GET /v1/providers/{provider_id}/usage-ledger`;
- receipt hashes derived from canonical job usage receipts;
- audit events when new usage ledger entries are appended.

## Measured Fields

Each receipt records:

- job, lease, provider, device, session, workload, GPU UUID, and terminal job status;
- lease start/end and job start/end timestamps;
- reserved GPU seconds;
- actual GPU seconds;
- billable and non-billable GPU seconds as metering basis only;
- idle billable and idle unbillable GPU seconds;
- input bytes, output bytes, network transfer bytes, and storage bytes from artifact metadata;
- retry count from sequenced job events;
- provider/customer failure classification when it can be derived from backend-constrained result metadata;
- challenge non-billable seconds, currently `0` for job receipts;
- reason codes explaining BN-15 policy decisions.

## Append-Only Rules

The table has one `job_usage_finalized` entry per job. Replaying finalize returns the existing entry with `duplicate=true` instead of mutating it.

A database trigger rejects `UPDATE` and `DELETE` on `usage_ledger_entries`, so later corrections must be modeled as future compensating ledger entries rather than edits. BN-15 only creates the first terminal job usage entry.

## Receipt Integrity

BN-15 stores a canonical `receipt_hash` and a `source_hash` over the job/lease source state. Signature fields are present in the contract, but `receipt_signature_status` is `hash_only_backend_signature_not_configured` until a backend receipt signing key is introduced.

## Deferred

BN-15 intentionally leaves out:

- customer balances, provider payable balances, invoices, and double-entry accounting;
- Pix/payment gateway integration;
- payouts, payout holds, chargeback reserves, refunds, and tax/KYC state;
- byte-level artifact transfer verification;
- storage retention charging;
- signed receipt key management;
- dispute workflows and manual adjustment entries;
- marketplace pricing and reservation billing.