# Provider Job Artifact Data Plane

## Status

The provider artifact data plane is implemented but deliberately disconnected
from production `remote-session connect`. It transfers declared job bytes while
keeping the workload container offline and unaware of transport credentials.

```text
Control Plane object storage
        |
        | HTTPS/loopback HTTP + job bearer credential
        v
Provider JobDataPlaneClient
        |
        | private per-job workspace
        v
trusted Burd helper -> bounded input tmpfs volume (read-only to workload)
        |
trusted offline anchor keeps both tmpfs mounts active for the job
        |
        v
offline workload -> bounded output tmpfs volume -> trusted Burd helper
        |
        v
Provider JobDataPlaneClient -> verified upload -> terminal result
```

## Agent lifecycle

After accepting an assignment, the worker:

1. creates a private workspace bound to `job_id` and `lease_id`;
2. downloads every declared input to a private temporary file;
3. enforces the declared size during streaming and verifies SHA-256;
4. atomically finalizes verified inputs;
5. starts a fixed, offline Burd anchor that keeps both named tmpfs mounts active
   until cleanup, then asks a digest-pinned helper to import inputs and mounts
   the input volume read-only in the workload;
6. runs the Linux container with `--network none`;
7. writes `/burd/output` to a separate bounded named tmpfs volume;
8. asks the helper to export that volume into its own staging layer, then uses
   structured `docker cp` only between the helper layer and private workspace;
9. rejects symlinks, nested/undeclared output files, excessive size, or missing
   outputs;
10. hashes and uploads each output with explicit `Content-Length`, then
    cross-checks the server receipt;
11. submits only those verified receipts and always attempts helper, volume,
    container, and workspace cleanup.

The raw job credential is held only by the Agent HTTP client. It is not added to
the container environment, arguments, labels, workspace, logs, or result
metrics. Artifact workloads do not persist workload stdout/stderr tails in job
metrics.

## Control Plane enforcement

The Control Plane exposes job-credential-protected GET/PUT endpoints for the
paths present in `JobDataPlaneGrant`. It validates credential hash and expiry,
job state, artifact direction, manifest binding, content length, content type,
and SHA-256. Uploads use private temporary files and atomic rename. PostgreSQL
migration `0025_job_artifact_transfers` stores only verified output metadata.

A successful `SubmitJobResult` must contain exactly the expected outputs and
must match the recorded artifact ID, object key, role, content type, size, and
SHA-256. Failed jobs cannot claim result artifacts. Terminal result or admin
cancellation clears the job credential hash and expiry.

## Limits and transport policy

- at most 32 inputs and 32 outputs;
- at most 10 GiB total input and 10 GiB total declared output;
- 64 KiB streaming buffers rather than whole-artifact buffering;
- HTTPS only, except loopback HTTP for local tests;
- redirects disabled;
- 30-second DNS, connect, request, upload, response, and download phase
  timeouts plus a separate 120-second whole-call ceiling and the authoritative
  job, lease, credential, and shutdown deadline;
- relative grant URLs only, without query strings, fragments, or embedded
  credentials;
- helper and workload images must be immutable/digest-pinned and already local;
- helper containers have fixed operations, no shell, no network, no customer
  paths, and bounded resources; all defaults are dropped and only import gets
  `DAC_READ_SEARCH` for private staged files inside its isolated namespace;
- no archive extraction, arbitrary host path, bind mount, or container network.

CI builds the minimal helper from the reviewed Rust source and runs a real
Docker roundtrip without NVIDIA. The gate proves that a private `0600` host
input is readable by workload UID `1000`, the input mount rejects writes, both
tmpfs mounts survive the import/workload/export transitions, the output is
exported byte-for-byte, and containers/volumes are removed.

## Deferred

This slice does not provide customer input upload, external S3-compatible or
signed-URL adapters, retention/garbage collection, malware scanning, runtime
proof, scheduler admission by verified capability, active-job remote
cancellation push, production worker wiring, or paid execution. The disconnected
provider worker now discovers cancellation through authenticated polling and
propagates it to this data plane.
