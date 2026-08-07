# Provider Job Runner Contract

## Status

This document describes the backend-authoritative execution contract returned by the Control Plane and the Agent-side provider job orchestration boundary. A Linux-native Docker/NVIDIA executor exists as an isolated Agent component, but production session wiring remains disabled.

Schema versions:

- `burd-provider-job-execution-v2`;
- `burd-provider-job-runtime-policy-v2`.

V2 removes the ambiguous `target_os` field. The job policy describes workload
requirements through `container_os=linux`, `gpu_backend=cuda`, and
`gpu_runtime=nvidia`; it does not require the provider's physical host to run
Linux.

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

The v2 policy requires:

- Docker-compatible execution of a Linux container;
- CUDA through the NVIDIA GPU runtime;
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

Host capability is a separate contract. `ProviderRuntimeCapability` reports
`host_os`, a local backend such as `docker_linux_native` or `docker_wsl2`, the
Linux container environment, GPU runtime, readiness, reason codes, and observed
GPU UUIDs. The Agent report is not trusted proof: the separate runtime
verification state remains `reported` with `gpu_uuid_binding=unverified` until
a future Control Plane proof succeeds.

## Current Boundary

Implemented:

- versioned protocol types and OpenAPI schemas;
- backend construction and validation of the assignment bundle;
- explicit transition validation with a fake executor in tests;
- canonical approved-template list shared by protocol and Control Plane;
- a separate Agent provider job worker and executor interface;
- authenticated polling, local bundle/session/GPU/expiry validation, acceptance, ordered `provisioning`, `running`, and `uploading` events, and terminal result submission;
- one active execution at a time per worker, bounded in-memory replay rejection, authoritative deadline cancellation, and cooperative Agent shutdown;
- deterministic fake-executor coverage for success, failure, invalid bundles, expiry, shutdown, and replay;
- integration-only session-supervisor wiring for the provider job worker.
- Runtime Platform Model v2, which separates provider host capability from the
  Linux-container job policy.
- `DockerNvidiaProviderJobExecutor` separated from the `DockerRuntimeBackend`
  interface, with `LinuxNativeDockerBackend` as the first implementation;
- exact template/image-digest authorization with no permissive default;
- read-only Docker/NVIDIA/GPU/image probes before side effects;
- structured `docker create`, `start`, `inspect`, bounded/redacted `logs`,
  `stop`, `kill`, and `rm --force` operations without a shell;
- deterministic container names, Burd ownership labels, controlled stale
  cleanup, resource limits, GPU UUID binding, timeout/cancellation, distinct
  exit/OOM failures, and mandatory removal;
- fake-backend unit coverage and an ignored physical Linux/NVIDIA isolation
  test.

The production `remote-session connect` command does not start the worker yet. The Linux executor is intentionally disconnected until Windows support, the real data plane, runtime verification, and controlled activation are complete.

Not implemented:

- byte-level artifact download or result upload;
- signed object-storage URLs;
- secret injection;
- remote cancellation discovery while an execution is active;
- Control Plane persistence or verification of reported runtime capabilities;
- scheduler filtering by verified runtime capability;
- the Windows WSL2 Docker backend;
- production worker/executor wiring;
- paid workload execution.

The current worker revalidates the complete bundle before acceptance and reports persisted transitions through the existing authenticated job endpoints. Its cancellation token currently represents local shutdown and authoritative assignment deadlines only. `POST /v1/jobs/{job_id}/cancel` remains administrative, and the remote control protocol has no typed `job_cancel` command or provider-authenticated job-status query; tests must not describe this boundary as remote cancellation.
