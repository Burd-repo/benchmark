# BN-12 - Secure Provider Runtime

BN-12 defines the local runtime capability and sandbox-planning boundary for
paid compute without executing customer jobs. Runtime Platform Model v2 keeps
the provider host separate from the Linux container required by Burd
workloads.

## Contracts

- `burd-provider-runtime-capability-v1` describes what the Agent observes on
  the local machine.
- `burd-provider-runtime-verification-v1` keeps Agent reports separate from
  future Control Plane verification.
- `burd-secure-runtime-v2` describes how an approved workload would be
  executed locally.
- `burd-secure-runtime-policy-v2` versions the secure planning policy.

The v1 `target_os` field was removed rather than reinterpreted. In v1 it meant
the physical host in `SecureRuntimePlan`, while the job runtime policy used the
same name for the Linux workload environment.

## Host, Capability, And Plan

The three concepts are intentionally different:

```text
ProviderRuntimeCapability
    What this machine reports it can offer.

SecureRuntimePlan
    How the Agent plans to execute one approved workload.

ProviderJobRuntimePolicy
    What the backend requires from every provider runtime.
```

Burd workloads remain Linux containers with CUDA and NVIDIA. The provider host
may be Linux or Windows:

```text
Linux host   -> docker_linux_native -> Linux container
Windows host -> docker_wsl2         -> Linux container
```

`runtime_provider` is optional diagnostic metadata such as `docker_desktop` or
`docker_engine`. It is not a workload requirement, so the `docker_wsl2`
contract does not bind Burd to Docker Desktop.

## Capability Statuses

- `ready`: the local probes observed the required runtime components.
- `not_ready`: the host may be supported, but one or more components are
  missing or still require backend verification.
- `unsupported`: no Burd runtime backend is defined for the host platform.

`reason_codes` preserve actionable causes such as `docker_unavailable`,
`wsl2_runtime_unavailable`, `nvidia_runtime_unavailable`,
`gpu_uuid_unavailable`, and `runtime_backend_verification_required`.

Windows is not globally unsupported. In this slice, a detected
`docker_wsl2` backend remains `not_ready` with
`runtime_backend_verification_required` until the Windows backend and its
physical NVIDIA isolation test are implemented.

## Reported Versus Verified

Agent output is never proof by itself. Local plans currently emit:

```json
{
  "authority": "agent",
  "status": "reported",
  "gpu_uuid_binding": "unverified",
  "reason_codes": ["runtime_proof_required"]
}
```

Only a future Control Plane runtime-proof flow may produce
`authority=control_plane`, `status=verified`, and
`gpu_uuid_binding=verified`. PR #82 does not persist capability reports, make
them scheduler-authoritative, or change scheduler candidate filtering.

## Agent Commands

```bash
burd-agent runtime check --json
```

`runtime check` reports the local capability and returns a plan without an
image reference. The plan is diagnostic and does not authorize paid work.

```bash
burd-agent runtime plan \
  --image-ref ghcr.io/burd/runtime/llm@sha256:<digest> \
  --allow-image-ref ghcr.io/burd/runtime/llm@sha256:<digest> \
  --gpu-uuid GPU-... \
  --template-id llm_inference \
  --json
```

`runtime plan` returns `status=ready` only when the reported runtime
capability, image, allowlist, observed GPU UUID, resource limits, and security
profile pass. A caller-supplied GPU UUID that was not observed locally fails
closed.

## Approved Initial Templates

- `llm_inference`
- `embeddings`
- `image_generation`
- `whisper_transcription`
- `file_processing`

The approved template list remains narrower than arbitrary container
execution. Customer-provided commands and entrypoint overrides are forbidden.

## Plan Statuses

- `ready`: the local capability and workload-specific plan checks passed.
- `verification_required`: required workload binding data, such as an image,
  is missing.
- `blocked`: a capability, security, image, GPU binding, or resource check
  failed.

Capability readiness and plan readiness are local observations. Neither is
backend verification or global provider eligibility.

The shared `validate_provider_runtime_compatibility()` helper compares the job
policy with a locally ready capability. It is deliberately non-authoritative:
scheduler admission must later require a persisted Control Plane verification
record as an additional condition.

## Security Defaults

The plan keeps the existing fail-closed Linux-container policy:

- digest-pinned, allowlisted images;
- `--pull never`;
- one observed GPU UUID through `--gpus device=<GPU UUID>`;
- read-only root filesystem;
- user `1000:1000`;
- all capabilities dropped;
- `no-new-privileges` and default seccomp;
- explicit CPU, memory, PID, shared-memory, network, and IPC limits;
- explicit tmpfs mounts;
- ephemeral secrets and mandatory cleanup;
- no arbitrary command or entrypoint override.

The Windows backend must not expose arbitrary Windows paths, user profiles,
Desktop, Documents, AppData, or entire home directories to a workload. Its
storage boundary will use Docker-managed or WSL-local isolated storage.

## Current Boundary

PR #82 defines contracts, local detection, validation, JSON serialization,
OpenAPI components, documentation, and tests. It does not:

- launch Docker containers;
- implement the Linux or Windows executor backend;
- prove GPU UUID isolation through WSL2;
- upload capability reports to the Control Plane;
- persist a verified runtime state;
- filter scheduler candidates by runtime capability;
- transfer customer artifacts or results;
- activate production provider jobs.

The production worker remains disabled until the executor, data plane,
runtime verification, and controlled activation boundaries are complete.
