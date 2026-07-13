# BN-12 - Secure Provider Runtime

BN-12 prepares the first secure runtime boundary for paid compute without yet
accepting customer jobs. The agent can inspect the local host and build a
versioned Docker/NVIDIA sandbox plan, but no arbitrary workload is executed in
this stage.

## Scope

- Define the `SecureRuntimePlan` protocol contract.
- Add `burd-agent runtime check --json` for local runtime readiness diagnosis.
- Add `burd-agent runtime plan --json` for an approved runtime image.
- Require digest-pinned image references through `@sha256:`.
- Require explicit image allowlisting before producing executable Docker args.
- Bind planned execution to a specific GPU UUID.
- Require Linux for ready runtime status.
- Require Docker plus the NVIDIA container runtime.
- Model read-only root filesystem, non-root user, dropped capabilities,
  no-new-privileges, seccomp, explicit CPU/RAM/PID/shm limits, no network, no
  IPC sharing, explicit tmpfs mounts, ephemeral secrets, and mandatory cleanup.

## Non-Goals

- No job API.
- No scheduler lease.
- No customer artifact download.
- No result upload.
- No shell command execution.
- No container launch from the backend.
- No billing, metering, marketplace listing, or payout.
- No Kubernetes, distributed inference, or multi-provider job execution.

## Agent Commands

```bash
burd-agent runtime check --json
```

`runtime check` probes the host and returns a `SecureRuntimePlan` with no image
reference. It is diagnostic. On Windows and macOS the expected status is
`unsupported_host` because BN-12 starts with Linux providers.

```bash
burd-agent runtime plan \
  --image-ref ghcr.io/burd/runtime/llm@sha256:<digest> \
  --allow-image-ref ghcr.io/burd/runtime/llm@sha256:<digest> \
  --gpu-uuid GPU-... \
  --template-id llm_inference \
  --json
```

`runtime plan` returns `status=ready` only when the host, image, allowlist, GPU
binding, resources, and security profile all pass. Docker arguments are emitted
only for `ready` plans.

## Approved Initial Templates

- `llm_inference`
- `embeddings`
- `image_generation`
- `whisper_transcription`
- `file_processing`

The approved template list is intentionally narrower than arbitrary container
execution. BN-13 can map job templates to these runtime templates.

## Plan Statuses

- `ready`: the runtime plan can be bound to a future backend lease.
- `verification_required`: the host is reachable but required execution binding
  data, such as image or GPU UUID, is missing.
- `blocked`: a hard security or runtime requirement failed.
- `unsupported_host`: the current OS is not supported for BN-12 runtime
  readiness.

## Security Defaults

The plan uses Docker with NVIDIA Container Toolkit expectations:

- `--pull never`
- `--gpus device=<GPU UUID>`
- `--read-only`
- `--user 1000:1000`
- `--cap-drop ALL`
- `--security-opt no-new-privileges`
- `--security-opt seccomp=default`
- `--pids-limit 512`
- `--memory 8192m` by default
- `--cpus 4` by default
- `--network none`
- `--ipc none`
- `--shm-size 64m`
- tmpfs `/tmp` with `rw,noexec,nosuid,nodev,size=1024m`
- tmpfs `/run/burd-secrets` with `rw,noexec,nosuid,nodev,size=16m,mode=0700`

The plan does not include an entrypoint override or customer shell command.

## Authority Boundary

BN-12 is still agent-local runtime preparation. The backend does not yet trust a
provider because it produced a local runtime plan. A future job lease must bind:

- provider ID;
- device ID;
- session ID;
- workload template;
- image digest;
- GPU UUID;
- policy version;
- lease ID;
- job-specific credentials;
- metering receipt.

Until BN-13 and BN-14 exist, secure runtime output is evidence of local
capability, not authorization to run paid work.

## BN-13 Handoff

BN-13 should consume this contract when implementing the first Job API and data
plane. The scheduler and job service should never send arbitrary shell payloads
to the provider. They should select an approved template, an allowlisted digest,
a GPU UUID, a lease, signed URLs, and ephemeral credentials.