# BN-13 - Job API And Data Plane

BN-13 introduces the backend-owned compute job registry and data-plane contract. It creates jobs for a specific provider, device, and active remote session, lets the provider pull the next assignment over authenticated session credentials, records sequenced progress events, transfers declared provider-side artifacts, and accepts final result metadata.

BN-13 does not implement customer artifact ingress, external object-storage signing, marketplace reservations, billing, arbitrary shell execution, or paid workload execution. It is the control-plane foundation that BN-14 leases and BN-15 metering can consume.

## Backend Scope

BN-13 adds:

- `compute_jobs`, an appendable job metadata and state table;
- `job_events`, a sequenced provider progress event table;
- `job_artifact_uploads`, a registry of output bytes verified by the Control Plane;
- `burd-protocol` job, artifact, event, result, cancel, list, next-job, and data-plane grant contracts;
- admin job creation with `Idempotency-Key` replay protection;
- provider pull of the next queued job using an authenticated remote session;
- job-scoped data-plane grants with separate credentials and scoped artifact paths;
- accept, progress event, result, list, read, and cancel endpoints;
- audit events for create, assign, accept, result, and cancel transitions.

The first implementation requires:

- provider and device exist and are not blocked;
- device status is active;
- session status is `online` or `degraded`;
- backend workload eligibility is `eligible` or `limited`;
- backend is `cuda`;
- template is one of `llm_inference`, `embeddings`, `image_generation`, `whisper_transcription`, or `file_processing`;
- runtime image reference is digest-pinned with `@sha256:`;
- job parameters, artifact metadata, result metrics, and event metadata do not contain obvious secret fields.

## API

Admin endpoints:

- `POST /v1/jobs` creates one queued job for a specific provider, device, session, workload, template, image digest, GPU UUID, and artifact manifest.
- `GET /v1/jobs/{job_id}` reads job metadata and status.
- `GET /v1/providers/{provider_id}/jobs` lists provider job metadata.
- `POST /v1/jobs/{job_id}/cancel` cancels a non-terminal job.

Provider-session endpoints:

- `GET /v1/sessions/{session_id}/jobs/next` atomically assigns the oldest queued job for that provider/device/session and returns a job-scoped data-plane grant.
- `POST /v1/sessions/{session_id}/jobs/{job_id}/accept` acknowledges assignment before provisioning.
- `POST /v1/sessions/{session_id}/jobs/{job_id}/events` appends a sequenced progress event and may move the job to `provisioning`, `running`, or `uploading`.
- `POST /v1/sessions/{session_id}/jobs/{job_id}/result` submits final `succeeded` or `failed` result metadata and output artifact references.

Job-credential endpoints:

- `GET /v1/jobs/{job_id}/artifacts/{artifact_id}/download` streams one declared input.
- `PUT /v1/jobs/{job_id}/results/{artifact_id}/upload` streams and records one declared output.

## Data Plane Contract

The data-plane grant includes:

- `schema_version: burd-job-data-plane-grant-v1`;
- `job_id`;
- an opaque job credential returned only to the assigned session;
- `credential_expires_at` calculated from job timeout plus server grace period;
- scoped download paths for declared input artifacts;
- scoped upload paths for declared expected outputs.

The paths do not embed the credential. The Agent sends it only in the
`Authorization` header, never forwards it to the workload container, and
accepts no redirects. Input and output bytes are streamed with exact size and
SHA-256 verification. Uploads finalize atomically and terminal success is
accepted only when every declared output matches a verified upload record.

The current transport is a Control Plane-owned filesystem object-store adapter.
Customer input ingestion, externally signed object-store URLs, retention, and
garbage collection remain separate production work.

## State Machine

```text
queued
-> assigned
-> accepted
-> provisioning
-> running
-> uploading
-> succeeded | failed | cancelled
```

BN-13 enforces non-terminal transitions for assignment, provider events, result submission, and cancellation. It prevents duplicate event sequences per job and prevents changing terminal job results.

## Deferred

BN-13 does not implement:

- scheduler selection or leases;
- provider-side container execution of jobs;
- arbitrary shell or customer-defined commands;
- customer-side artifact ingestion;
- object storage signed URL generation;
- usage metering or job receipts;
- marketplace listing, reservation, billing, Pix, payouts, refunds, or disputes;
- multi-GPU or multi-provider jobs.
