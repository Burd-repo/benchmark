# Provider Job Runner Contract

## Status

This document describes the backend-authoritative execution contract returned by the Control Plane and the Agent-side provider job orchestration boundary. A Linux-native Docker/NVIDIA executor exists as an isolated Agent component, but production session wiring remains disabled.

Schema versions:

- `burd-provider-job-execution-v3`;
- `burd-provider-job-runtime-policy-v2`.

V2 removed the ambiguous `target_os` field. The job policy describes workload
requirements through `container_os=linux`, `gpu_backend=cuda`, and
`gpu_runtime=nvidia`; it does not require the provider's physical host to run
Linux.

V3 adds a separate maximum Control Plane silence bound to the cancellation policy. It does not
change the runtime isolation policy.

## Assignment Bundle

`GET /v1/sessions/{session_id}/jobs/next` returns either no assignment or a complete bundle containing:

- `job`;
- `lease`;
- `data_plane`;
- `execution`.

Before constructing the bundle, the Control Plane re-evaluates Runtime Admission for the exact
provider/device/GPU inside the locked assignment transaction. Denied offers are expired and audited
without a credential; the bounded poll continues to a later offer. Only current admission can move
the job to `assigned`. The assignment may use a newer valid proof than the scheduler lease audit.

`execution` binds the job, lease, provider, device, authenticated session, workload type, approved template, digest-pinned image, GPU UUID, CUDA backend, workload policy, timeout, lease expiry, and data-plane credential expiry. The Control Plane validates the complete bundle before returning it. Partial bundles fail closed.

The execution specification never contains the raw data-plane credential, arbitrary command text, or an entrypoint override. The credential remains only in `data_plane` and must be redacted from logs; only its hash and expiry are persisted.

Acceptance is a separate authority gate. `POST /v1/sessions/{session_id}/jobs/{job_id}/accept`
includes the exact `lease_id` from the bundle, locks that job/lease pair, requires it to match the
persisted assignment binding and an unexpired `offered` lease, and re-evaluates Runtime Admission
before execution can begin. A stale acknowledgement returns `409` without touching a newer
assignment. If the current assignment loses authority, acceptance returns the job to `queued`,
clears its credential hash and expiry, terminalizes its lease, and records a non-secret
acceptance-withheld audit event. A successful accept updates exactly one offered lease and audits
the current Runtime Admission evidence in the same transaction.

After acceptance, the worker starts an assignment-scoped watcher against
`GET /v1/sessions/{session_id}/jobs/{job_id}/control?lease_id={lease_id}`. The Control Plane
returns only `continue` or `cancel`, bound to the authenticated session and exact persisted
assignment lease. Responses are non-cacheable. A stale lease receives `409` and cannot mutate or
cancel a newer assignment.

The watcher retries transport and server failures only until `max_control_silence_seconds`.
Session authority loss, missing or conflicting assignment authority, an invalid response contract,
an explicit cancel directive, or excessive Control Plane silence cancels local work fail closed.
The watcher starts immediately after acceptance and is stopped and joined before that assignment
returns, so it cannot survive into a later assignment.

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
- a separate bound for maximum Control Plane silence;
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
- authenticated polling, local bundle/session/GPU/expiry validation, backend-authoritative acceptance revalidation, ordered `provisioning`, `running`, and `uploading` events, and terminal result submission;
- one active execution at a time per worker, bounded in-memory replay rejection, authoritative deadline cancellation, and cooperative Agent shutdown;
- deterministic fake-executor coverage for success, failure, invalid bundles, expiry, shutdown, and replay;
- integration-only session-supervisor wiring for the provider job worker;
- exact-assignment active-job control polling, bounded Control Plane silence, and fail-closed local
  cancellation propagated through data-plane operations and the Docker executor;
- remote administrative cancellation cleanup without submitting a contradictory `failed` result.
- Runtime Platform Model v2, which separates provider host capability from the
  Linux-container job policy.
- `DockerNvidiaProviderJobExecutor` separated from the `DockerRuntimeBackend`
  interface, with `LinuxNativeDockerBackend` and
  `WindowsWsl2DockerBackend` sharing one Linux-container CLI runtime;
- exact template/image-digest authorization with no permissive default;
- read-only Docker/NVIDIA/GPU/image probes before side effects;
- structured `docker create`, `start`, `inspect`, bounded/redacted `logs`,
  explicit `TERM`/policy-deadline/`KILL` escalation, and `rm --force`
  operations without a shell;
- wall-clock and cancellation bounds for every Docker/NVIDIA CLI child, with
  termination/reaping before independent bounded cleanup;
- exact-name stale-container lookup that distinguishes absence from Docker
  list/inspect failure;
- deterministic container names, Burd ownership labels, controlled stale
  cleanup, resource limits, GPU UUID binding, timeout/cancellation, distinct
  exit/OOM failures, and mandatory removal;
- fake-backend unit coverage plus ignored physical Linux/NVIDIA and
  Windows/WSL2/NVIDIA gates for multi-GPU isolation, unavailable UUIDs,
  cancellation, force kill, timeout, cleanup, and exact CUDA proof binding;
- a separate `JobDataPlaneClient` boundary that downloads declared inputs into
  a private per-job workspace, verifies size and SHA-256 while streaming,
  uploads only declared outputs, and always attempts bounded cleanup;
- separate bounded tmpfs volumes for read-only workload inputs and UID-1000
  outputs, bridged through a digest-pinned, fixed-operation Burd helper without
  host bind mounts, workload network, shell, or job credentials;
- Control Plane artifact GET/PUT endpoints with job-scoped bearer
  authorization, bounded streaming, no redirects, atomic private-file writes,
  verified upload persistence, and terminal-result/upload consistency checks.

The production `remote-session connect` command does not start the worker yet.
Both platform backends and the artifact data plane remain intentionally
disconnected until commit-bound physical gate evidence is reviewed and
controlled activation is complete. Harness availability or an ignored test does
not count as physical verification.

Not implemented:

- signed object-storage URLs;
- customer input ingestion and production object-storage adapters;
- secret injection;
- Control Plane persistence or verification of reported runtime capabilities;
- direct scheduler trust in raw Agent-reported runtime capability remains intentionally absent;
  scheduler and assignment consume authoritative Runtime Admission instead;
- production worker/executor wiring;
- automatic WSL2, Docker, driver, or NVIDIA component installation;
- paid workload execution.

The current worker revalidates the complete bundle before acceptance and reports persisted
transitions through the existing authenticated job endpoints. `POST /v1/jobs/{job_id}/cancel`
remains administrative; the Agent discovers its terminal decision through authenticated polling,
not a WebSocket push command. Cancellation is cooperative during artifact transfer: progress checks
occur for each 64 KiB chunk. DNS, connect, request send, body send, response receive, and body
receive phases are each capped at 30 seconds, with a separate 120-second whole-call ceiling. A
failed post-cancellation workspace cleanup stops the worker instead of accepting another
assignment. Production worker activation remains disabled.
