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
Artifact inputs and outputs use separate named tmpfs volumes created by the
Docker local driver, which supports mount-style options on Docker Desktop. A
digest-pinned Burd anchor keeps the tmpfs mounts active while fixed helpers
import and export files through their own container layers; the workload sees
input read-only and output writable. `docker cp` never
targets a workload mount. The plan does not expose Windows drives, user
profiles, `/mnt/c`, `/mnt/d`, `\\wsl$`, the Docker socket, or any persistent
host path.

## Physical NVIDIA isolation test

Unit tests prove that both backends generate identical hardened Docker
arguments and that the Windows path contains no host mounts or shell. The
physical Windows test remains ignored until run on a suitable machine:

```powershell
$env:BURD_WINDOWS_WSL2_NVIDIA_TEST_IMAGE = 'registry/image@sha256:<64-hex-digest>'
$env:BURD_WINDOWS_WSL2_NVIDIA_LIFECYCLE_TEST_IMAGE = 'registry/image@sha256:<64-hex-digest>'
$env:BURD_WINDOWS_WSL2_NVIDIA_TEST_GPU_UUID = 'GPU-...'
cargo test -p burd-agent physical_windows_wsl2_nvidia_isolation_gate -- --ignored --nocapture --test-threads=1
cargo test -p burd-agent physical_windows_wsl2_nvidia_lifecycle_gate -- --ignored --nocapture --test-threads=1
```

The gate requires at least two host GPUs, leases one exact UUID, rejects
cross-GPU visibility and an unavailable UUID, and proves cancellation, force
kill, timeout, and cleanup. A separate runtime-proof test performs the real CUDA
operation. Until a reviewed physical workflow run exists, Windows remains
denied by Runtime Admission with `windows_physical_gate_required`; local Agent
capability remains reported rather than scheduler-authoritative. See
`physical-nvidia-gates.md`.

## Explicit non-goals

This slice does not install WSL2, Docker, drivers, or NVIDIA components. It does
not add a Burd-managed container engine, secret injection, runtime proof
persistence, scheduler admission, production worker wiring, or paid execution.
