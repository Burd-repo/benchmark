# Runtime Capability Verification

This slice turns an Agent-reported runtime capability into a time-limited, Control
Plane-authoritative verification. It does not make the scheduler consume that state and it does
not activate provider job execution.

## Trust boundary

`ProviderRuntimeCapability` remains an observation made by the Agent. A
`ProviderRuntimeVerificationRecord` is created only after the Control Plane issues a fresh
challenge and accepts a signed response bound to the enrolled device key.

The challenge binds all of the following:

- provider, device and current remote session;
- current hardware fingerprint and one exact GPU UUID from the persisted inventory;
- host OS and runtime backend (`docker_linux_native` or `docker_wsl2`);
- Linux container, CUDA and NVIDIA runtime requirements;
- proof policy and Agent runtime contract versions;
- a digest-pinned Burd proof image;
- nonce, issue time, expiry and verification TTL.

The administrator may request a shorter TTL, but cannot exceed the Control Plane policy. The
proof image is pinned by `BURD_CONTROL_RUNTIME_PROOF_IMAGE_REF`; the administrator-only issue
endpoint must use that exact configured digest. It never comes from a customer workload or from
the Agent response.

## Execution

The runtime verification worker is supervised with the existing control and proof workers. It is
enabled by the existing remote-session proof flag and is independent from the deliberately
disabled production provider-job worker.

The proof container uses the same Docker backend and command deadline infrastructure as compute:

```text
Control Plane challenge
        |
        v
Agent validates local identity, fingerprint and expiry
        |
        v
Docker backend read-only environment probes
        |
        v
digest-pinned proof image + exact GPU UUID
network=none, read-only, uid=1000:1000, cap-drop=ALL
        |
        v
container observes nonce and visible GPU UUIDs
        |
        v
Agent builds canonical fingerprint and signs response with Ed25519
        |
        v
Control Plane independently verifies and persists a TTL-bound record
```

The proof image entrypoint must emit one JSON document on stdout:

```json
{
  "schema_version": "burd-runtime-proof-output-v1",
  "nonce": "value from BURD_RUNTIME_PROOF_NONCE",
  "observed_gpu_uuids": ["GPU-..."],
  "nvidia_driver_version": "...",
  "cuda_runtime_version": "..."
}
```

The response is rejected unless `observed_gpu_uuids` is exactly the one-element array containing
the challenged GPU. A host with two GPUs therefore proves both selection and non-visibility of the
other GPU inside the container.

## Verification fingerprint

The canonical fingerprint includes:

- provider, device, hardware fingerprint and GPU UUID;
- host OS, runtime backend and Linux container isolation model;
- Docker server version, NVIDIA driver, NVIDIA runtime and CUDA runtime;
- Agent runtime contract and proof policy versions;
- digest-pinned proof image.

A later successful proof for the same provider/device/GPU supersedes the previous active record.
Records also expire by TTL. Runtime admission compares a recent signed runtime observation against
the re-observable admission fingerprint stored with the proof record. The record remains
device/GPU/runtime-bound across a session reconnect, but key rotation, hardware/runtime drift,
expiry, blocking or a missing current observation denies admission.

After key rotation, admission recovery requires a new signed GPU inventory, a new signed runtime
observation and a new runtime verification proof, all bound to the new active device key.

The result is named `runtime_verified`. It is a functional readiness/admission proof, not hardware
attestation: a provider controls its host OS, Agent process and local signing key. TPM/TEE-backed
integrity remains separate future work.

## Replay and failure behavior

- challenge nonces and response hashes are unique in PostgreSQL;
- only `issued` or `acknowledged` challenges accept a response;
- expired challenges are terminal;
- provider/device/session, fingerprint, GPU, backend, image, policy and nonce are verified again by
  the Control Plane;
- the response hash and Ed25519 signature are verified using the active enrolled device key;
- malformed, mismatched or replayed responses never create a verified record;
- audit metadata excludes the nonce and signature.

## Physical gate

Unit and integration tests can validate contracts and lifecycle without NVIDIA hardware. Before a
Windows or Linux backend is admitted for real jobs, a physical multi-GPU gate must prove:

```text
host GPUs:      GPU-A, GPU-B
challenge GPU: GPU-B
container:     GPU-B visible
               GPU-A not visible
```

Windows remains reported/not-ready until this gate passes through the complete
Windows -> WSL2 -> Linux Docker engine -> NVIDIA chain.

## Deferred work

This slice intentionally does not implement:

- scheduler consumption of runtime admission;
- production provider-job activation;
- WebSocket push for active-job cancellation;
- customer ingress, marketplace changes, billing or metering changes;
- proof image publishing or automatic runtime installation.

Those boundaries belong to the following admission and controlled-activation slices.
