# Provider Job Canary Activation

## Status

The Provider Job Worker is disabled by default. This canary is the first
production-shaped wiring of the existing Remote Session, authoritative job
gates, HTTP artifact data plane, and Linux Docker/NVIDIA executor. It is not a
physical NVIDIA gate pass, production-readiness declaration, customer workload
path, or paid execution path.

Windows and WSL2 remain disconnected and fail closed under
`windows_physical_gate_required`.

## Explicit activation

The operator must provide all canary inputs on every foreground start:

```bash
burd-agent remote-session connect \
  --proofs \
  --provider-job-worker-mode canary \
  --provider-job-image 'llm_inference=ghcr.io/burd/canary@sha256:<64-hex-digest>' \
  --provider-job-artifact-helper-image 'ghcr.io/burd/artifact-helper@sha256:<64-hex-digest>'
```

`--provider-job-image` may be repeated for up to 32 exact pairs. Templates must
belong to the protocol's canonical approved-template list. Duplicate pairs,
mutable tags, unapproved templates, missing helper image, missing `--proofs`, or
a non-Linux host are invalid invocation errors.

Omitting `--provider-job-worker-mode canary` preserves the previous behavior:
the session supervisor receives no provider-job runtime and makes no
`/jobs/next` request. Image options are rejected while the mode is disabled so
stale arguments cannot look active.

## Startup preflight

Canary startup is read-only and fail closed. Before the Remote Session begins,
the Agent requires:

1. a native Linux host and Linux Docker server;
2. the Docker NVIDIA runtime;
3. successful `nvidia-smi` inventory with at least one GPU;
4. the exact digest-pinned artifact-helper image already local;
5. every allowlisted workload image already local.

The Agent never pulls, installs, or substitutes an image, runtime, driver, or
executor. A failed preflight terminates with the redacted
`provider_job_canary_runtime` failure kind. No fallback executor exists.

After preflight, the worker uses `LocalProviderJobControlPlane`,
`HttpProviderJobDataPlane`, `DockerNvidiaProviderJobExecutor`,
`LinuxNativeDockerBackend`, and `StaticProviderJobImagePolicy`. It remains
serial: one assignment can execute at a time.

## Authority and kill switch

The local mode only permits the worker to participate. It does not grant a job.
Runtime Admission, scheduler lease, assignment revalidation, exact-lease
acceptance, local GPU UUID matching, and active execution control remain
authoritative.

The Control Plane can stop useful work by denying Runtime Admission, ceasing
assignments, revoking the session/device, or cancelling the exact active job.
The active cancellation watcher propagates that decision through HTTP transfer,
Docker termination, forced kill when required, and cleanup. The operator can
also stop the foreground Agent and restart without canary mode.

Local startup emits a bounded JSON event with `worker_mode=canary`, allowlist
count, and GPU count. Job acceptance persists the status message `provider
canary worker accepted assignment`. Neither event includes image values,
credentials, tokens, job credentials, or authorization headers.

## Financial and product boundary

Current compute jobs are created through the admin-authorized control path and
are not bound to customer reservations, billing settlement, or provider payout
authority. The canary does not change those contracts. Usage metering may record
technical execution facts, but it does not make a canary job billable or
payable.

Customer input artifact ingress now feeds backend-owned provider manifests, but
external object storage, paid scheduling, billing, payouts, and production
promotion remain separate work.

## Verification boundary

Unit and integration tests prove disabled-versus-enabled supervisor behavior,
configuration validation, authoritative job lifecycle, cancellation, bounded
I/O, Docker command construction, and cleanup. No physical NVIDIA execution was
performed on the AMD development host. The ignored/manual physical gates remain
the only path to permanent hardware evidence.
