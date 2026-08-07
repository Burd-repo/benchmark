# Provider Job Runner Contract

## Status

This document describes the implemented backend-authoritative execution contract returned by the Control Plane. It does not describe an implemented provider worker or container executor.

Schema versions:

- `burd-provider-job-execution-v1`;
- `burd-provider-job-runtime-policy-v1`.

## Assignment Bundle

`GET /v1/sessions/{session_id}/jobs/next` returns either no assignment or a complete bundle containing:

- `job`;
- `lease`;
- `data_plane`;
- `execution`.

`execution` binds the job, lease, provider, device, authenticated session, workload type, approved template, digest-pinned image, GPU UUID, CUDA backend, workload policy, timeout, lease expiry, and data-plane credential expiry. The Control Plane validates the complete bundle before returning it. Partial bundles fail closed.

The execution specification never contains the raw data-plane credential, arbitrary command text, or an entrypoint override. The credential remains only in `data_plane` and must be redacted from logs.

## Execution States

The contract defines these provider-runner states:

```text
assigned
-> accepted | failed | cancelled | expired
accepted
-> provisioning | failed | cancelled | expired
provisioning
-> running | failed | cancelled
running
-> uploading | succeeded | failed | cancelled
uploading
-> succeeded | failed | cancelled
```

`succeeded`, `failed`, `cancelled`, and `expired` are terminal. The existing backend job and lease endpoints remain authoritative for persisted state transitions; the enum does not create a second state registry.

## Runtime Policy

The v1 policy requires:

- Linux and Docker;
- commands sourced only from an approved template;
- no command or entrypoint override;
- digest-pinned image;
- explicit GPU UUID and CUDA backend;
- no container network;
- read-only root filesystem;
- non-root user;
- `no-new-privileges`;
- default seccomp profile;
- all capabilities dropped;
- bounded CPU, memory, PIDs, and shared memory;
- bounded cancellation polling, graceful stop, and forced termination;
- container, working-directory, ephemeral-secret, and credential cleanup.

The shared validator rejects schema, identity, lease, workload, policy, artifact-path, expiry, credential-shape, or runtime-policy mismatches.

## Current Boundary

Implemented:

- versioned protocol types and OpenAPI schemas;
- backend construction and validation of the assignment bundle;
- explicit transition validation with a fake executor in tests;
- canonical approved-template list shared by protocol and Control Plane.

Not implemented:

- Agent daemon polling and execution loop;
- Docker/containerd process execution;
- NVIDIA Container Toolkit integration in a remote worker;
- byte-level artifact download or result upload;
- signed object-storage URLs;
- secret injection;
- runtime cancellation or cleanup enforcement;
- paid workload execution.

A future provider runner must revalidate the complete bundle before provisioning and must report persisted transitions through the existing authenticated job endpoints.
