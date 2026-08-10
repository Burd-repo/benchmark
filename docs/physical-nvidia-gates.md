# Physical NVIDIA Gates

## Status

The repository contains reproducible, ignored physical gates for Linux NVIDIA
and Windows host -> WSL2 Docker Linux -> NVIDIA GPU-PV. The gates are separate
from normal CI and require dedicated self-hosted runners with at least two
physical NVIDIA GPUs.

The harness being present is not evidence that a platform passed. A platform is
physically validated only when the manual workflow succeeds for the evaluated
commit and retains its sanitized evidence artifact. No successful Linux or
Windows NVIDIA result was produced on the AMD development host used to add the
harness.

Windows therefore remains denied by Runtime Admission with
`windows_physical_gate_required`, and `remote-session connect` still does not
start the production Provider Job Worker.

## Gate Contract

Each platform gate requires:

- a dedicated runner with at least two visible physical NVIDIA GPUs;
- host `nvidia-smi`, a Linux Docker engine, and the NVIDIA container runtime;
- WSL2 and NVIDIA GPU-PV as additional Windows requirements;
- one selected GPU UUID that exists in the host inventory;
- an isolation image, lifecycle image, and runtime-proof image, all pre-pulled
  and addressed by immutable repository digest;
- no mutable tag fallback and no image pull by the Agent.

The isolation test proves:

1. the host inventory contains the selected GPU and at least one unleased GPU;
2. the container sees the selected UUID;
3. the container reports exactly one `GPU-` entry;
4. no other host GPU UUID appears in the container output;
5. a nonexistent UUID fails before container creation;
6. success and rejection leave no assignment container behind.

The lifecycle test waits until the real container is running, requests
cancellation, exercises bounded `TERM` -> `KILL`, and requires cleanup. It then
runs the same stubborn image with a two-second execution timeout and again
requires cleanup.

The runtime-proof test separately executes the digest-pinned proof image and
requires a real CUDA proof bound to exactly the selected GPU on a multi-GPU
host.

## Test Images

`tools/physical-nvidia-gate` defines the non-production isolation and lifecycle
images. Build both from an explicitly digest-pinned CUDA base, push them to the
controlled registry, resolve their repository digests, and pre-pull those exact
references on the runner. See the directory README for commands.

The runtime-proof image is separate because it implements the versioned CUDA
proof output contract rather than the test-only `report` or `stubborn` modes.

## Manual Workflow

Dispatch `.github/workflows/real-hardware-integration.yml` with `confirm=RUN`
from `main` and select one target. Jobs refuse to run from any other ref so a
public branch cannot supply code to a self-hosted hardware runner:

- `linux-nvidia` targets `[self-hosted, Linux, X64, burd-nvidia-linux]`;
- `windows-wsl2-nvidia` targets
  `[self-hosted, Windows, X64, burd-nvidia-windows]`;
- `detection` preserves the existing generic Windows hardware test.

Configure these non-secret variables in the protected `real-hardware`
environment:

```text
BURD_LINUX_NVIDIA_TEST_IMAGE
BURD_LINUX_NVIDIA_LIFECYCLE_TEST_IMAGE
BURD_LINUX_NVIDIA_TEST_GPU_UUID
BURD_WINDOWS_WSL2_NVIDIA_TEST_IMAGE
BURD_WINDOWS_WSL2_NVIDIA_LIFECYCLE_TEST_IMAGE
BURD_WINDOWS_WSL2_NVIDIA_TEST_GPU_UUID
BURD_RUNTIME_PROOF_IMAGE_REF
```

Only variables for the selected platform are required, plus the shared runtime
proof image. The workflow does not print or persist raw GPU UUIDs. Its artifact
contains the commit, platform, GPU count, SHA-256 of the normalized inventory
and selected UUID, host/WSL kernel, Docker/driver/Rust versions, image digests,
test logs, and `result=passed` only after every physical test succeeds.

## Promotion Rule

A passing artifact is review evidence, not an automatic policy override. Any
later removal of `windows_physical_gate_required` must be an explicit reviewed
code change that references accepted Windows evidence and keeps the remaining
Runtime Admission checks intact. No environment variable or administrative
override bypasses this rule.

Production worker activation remains a separate change after both required
platform evidence and the active-job I/O-stall bound are accepted.
