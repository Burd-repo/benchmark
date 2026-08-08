# Windows WSL2 Docker Backend

## Status

`WindowsWsl2DockerBackend` is an isolated implementation of
`DockerRuntimeBackend`. It is not wired to production `remote-session connect`,
does not install or configure host software, and does not make Windows runtime
capability scheduler-authoritative.

```text
DockerNvidiaProviderJobExecutor
              |
              v
      DockerRuntimeBackend
        |             |
        v             v
LinuxNative       WindowsWsl2
```

Both backends delegate Docker lifecycle operations to the same structured CLI
runtime and therefore produce the exact same Linux-container plan. The Windows
backend changes only host/runtime verification and reports
`runtime_backend=docker_wsl2`.

## Fail-closed probes

Before container creation on Windows, the backend:

1. requires a Windows host;
2. runs the structured probe `wsl.exe --system uname -r` and requires a WSL2
   kernel identifier;
3. requires the Docker server OS to be Linux;
4. requires Docker's engine kernel to identify the WSL2 backend;
5. requires the NVIDIA runtime to be advertised;
6. requires the leased GPU UUID to be present in host `nvidia-smi` output;
7. requires the digest-pinned image to be present locally.

Every probe uses `DockerCommandControl`, so job cancellation, lease/job
deadlines, the per-command cap, child termination, and reaping behave exactly as
they do for the Linux backend. No probe invokes a shell. `wsl.exe` is used only
for controlled detection; all Docker lifecycle calls execute directly through
the Windows Docker CLI connected to the Linux engine.

## Filesystem boundary

The Windows and Linux backends share the same no-bind-mount container contract.
Artifact inputs use an anonymous Docker-managed volume and outputs use bounded
tmpfs. The Agent transfers them with structured `docker cp`; no workspace path
enters Docker arguments. The plan does not expose Windows drives, user
profiles, `/mnt/c`, `/mnt/d`, `\\wsl$`, the Docker socket, or any persistent
host path.

## Physical NVIDIA isolation test

Unit tests prove that both backends generate identical hardened Docker
arguments and that the Windows path contains no host mounts or shell. The
physical Windows test remains ignored until run on a suitable machine:

```powershell
$env:BURD_WINDOWS_WSL2_NVIDIA_TEST_IMAGE = 'registry/image@sha256:<64-hex-digest>'
$env:BURD_WINDOWS_WSL2_NVIDIA_TEST_GPU_UUID = 'GPU-...'
cargo test -p burd-agent physical_windows_wsl2_nvidia_container_sees_only_leased_gpu -- --ignored --nocapture
```

The strongest gate uses at least two host GPUs, leases GPU-B, and succeeds only
when the container log contains GPU-B and exactly one `GPU-` entry. Until that
physical test and Control Plane verification exist, Windows capability remains
`not_ready`, `authority=agent`, `status=reported`, and
`gpu_uuid_binding=unverified`.

## Explicit non-goals

This slice does not install WSL2, Docker, drivers, or NVIDIA components. It does
not add a Burd-managed container engine, secret injection, runtime proof
persistence, scheduler admission, production worker wiring, or paid execution.
