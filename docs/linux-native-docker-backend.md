# Linux Native Docker Backend

## Status

This document records the isolated Linux-native executor boundary introduced
after Runtime Platform Model v2. The implementation is intentionally not wired
to production `remote-session connect`.

```text
ProviderJobExecutor
        |
        v
DockerNvidiaProviderJobExecutor
        |
        v
DockerRuntimeBackend
        |
        `-- LinuxNativeDockerBackend
```

`DockerNvidiaProviderJobExecutor` owns assignment validation, exact image
authorization, deadlines, cancellation, monitoring, result classification, and
cleanup. `LinuxNativeDockerBackend` only translates typed plans and lifecycle
operations to the Docker CLI.

## Fail-closed sequence

Before container creation, the executor:

1. revalidates the complete job, lease, data-plane, and execution bundle;
2. rejects malformed runtime identifiers;
3. delegates host-specific readiness to the selected runtime backend (this
   implementation identifies itself as `docker_linux_native`);
4. requires an exact locally approved `(template_id, image_ref)` pair;
5. rejects expired lease or data-plane deadlines;
6. verifies a Linux Docker server, the NVIDIA runtime, the leased GPU UUID via
   `nvidia-smi`, and the digest-pinned image already present locally;
7. removes an existing same-name container only when every Burd ownership label
   matches the current assignment.

An inspectable foreign name conflict is never started or removed.

## Container lifecycle

The backend uses structured process arguments and never invokes `sh -c` or
another shell:

```text
docker image inspect
docker volume create --driver local --opt type=tmpfs ...
docker create <trusted artifact helper> hold
docker start <anchor>                 # keeps both tmpfs mounts active
docker create <trusted artifact helper> import
docker cp <private-input-directory>/. <helper>:/burd/staging
docker start <helper>
docker rm --force <helper>
docker create --pull never ...
docker start
docker container inspect
docker logs --tail 200
docker create <trusted artifact helper> export
docker start <helper>
docker cp <helper>:/burd/staging/. <private-output-directory>
docker rm --force <helper>
docker rm --force <anchor>
docker kill --signal TERM
docker container inspect    # poll through the graceful window
docker kill --signal KILL   # at force_kill_after_seconds if still running
docker rm --force
```

The container is not created with `docker run --rm`. Its state remains
inspectable until the executor records exit/OOM data, collects bounded and
redacted log tails, and performs explicit cleanup.

Every Docker CLI and `nvidia-smi` invocation has a wall-clock deadline. Normal
execution commands are also interrupted by the job cancellation token and are
capped by the earlier lease/job deadline. A timed-out or cancelled CLI child is
terminated and reaped before the executor continues to cleanup. Cleanup uses
independent bounded commands so a cancellation cannot prevent removal.

Cancellation sends `TERM` without delegating escalation to `docker stop`.
The executor polls container state through `graceful_stop_seconds`, continues
the bounded escalation window, and sends explicit `KILL` at
`force_kill_after_seconds` only if the container remains running. Container
lookup uses a successful filtered list plus exact-name matching; absence is
`None`, while Docker list/inspect failures are errors and therefore fail closed.

The fixed security boundary includes:

- only `--gpus device=<leased GPU UUID>`;
- read-only root filesystem and user `1000:1000`;
- `cap-drop=ALL`, `no-new-privileges`, and Docker's built-in default seccomp
  profile (`seccomp=builtin` at the CLI boundary);
- no network or shared IPC;
- explicit CPU, memory, PID, shared-memory, and tmpfs limits;
- no restart policy or image-defined health check;
- no privileged mode, host namespaces, host bind mounts, Docker socket,
  arbitrary command, or entrypoint override.

Artifact jobs receive inputs and write outputs through separate, named,
size-limited tmpfs volumes. The input volume is mounted read-only at
`/burd/input`; its files are root-owned `0444`, so workload UID `1000` can read
but not modify them. The output volume is owned by UID/GID `1000` and mounted at
`/burd/output`. A digest-pinned Burd anchor keeps both local-driver tmpfs mounts
active across helper/workload transitions; fixed import/export helpers move
bytes through their own container layers. `docker cp` never targets a workload
mount, and no private host
path is included in the workload container plan or mounted into any container.

The raw data-plane credential is never copied into `DockerContainerPlan`,
labels, Docker arguments, logs, or metrics.

## Physical NVIDIA isolation test

The normal CI suite uses a fake backend and never requires Docker or NVIDIA.
The ignored physical test requires Linux, Docker Engine, NVIDIA Container
Toolkit, two or more visible GPUs for the strongest assertion, and a
digest-pinned image whose default command prints `nvidia-smi -L`.

```bash
export BURD_LINUX_NVIDIA_TEST_IMAGE='registry/image@sha256:<64-hex-digest>'
export BURD_LINUX_NVIDIA_TEST_GPU_UUID='GPU-...'
cargo test -p burd-agent physical_linux_nvidia_container_sees_only_leased_gpu -- --ignored --nocapture
```

The test succeeds only when the log contains the leased UUID and exactly one
`GPU-` entry. A physical multi-GPU run is still required before claiming
production GPU-isolation verification.

## Explicit non-goals

This Linux-specific slice does not describe the separate Windows WSL2 backend;
see `windows-wsl2-docker-backend.md`. Neither backend implements secret
injection, Control Plane runtime proof, scheduler admission, active-job remote
cancellation discovery, production worker wiring, or paid execution. Customer
input ingress and external object-storage adapters are also deferred.
