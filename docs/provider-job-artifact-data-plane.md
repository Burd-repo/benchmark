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
Docker-managed input volume -> offline container -> bounded output tmpfs
        |
        | structured docker cp
        v
Provider JobDataPlaneClient -> verified upload -> terminal result
```

## Agent lifecycle

After accepting an assignment, the worker:

1. creates a private workspace bound to `job_id` and `lease_id`;
2. downloads every declared input to a private temporary file;
3. enforces the declared size during streaming and verifies SHA-256;
4. atomically finalizes verified inputs;
5. asks the Docker backend to copy inputs into an anonymous volume;
6. runs the Linux container with `--network none`;
7. copies `/burd/output` from bounded tmpfs into the private workspace;
8. rejects symlinks, nested/undeclared output files, excessive size, or missing
   outputs;
9. hashes and uploads each output, then cross-checks the server receipt;
10. submits only those verified receipts and always attempts workspace cleanup.

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
- one global connection/read/operation timeout plus the authoritative job,
  lease, credential, and shutdown deadline;
- relative grant URLs only, without query strings, fragments, or embedded
  credentials;
- no archive extraction, arbitrary host path, bind mount, or container network.

## Deferred

This slice does not provide customer input upload, external S3-compatible or
signed-URL adapters, retention/garbage collection, malware scanning, runtime
proof, scheduler admission by verified capability, active-job remote
cancellation discovery, production worker wiring, or paid execution.
