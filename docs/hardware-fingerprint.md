# Hardware Fingerprint

Run:

```sh
burd-agent fingerprint --json
```

The command emits a versioned canonical hardware payload, its SHA-256
fingerprint, and the current marketplace GPU policy snapshot.

## Contract

`hardware_fingerprint` uses the format `sha256:<hex>` over the serialized
`burd-hardware-fingerprint-v1` payload. GPU entries are normalized and sorted so
enumeration order does not change the result.

The payload includes relevant stable signals:

- OS and architecture;
- CPU name and core count;
- total RAM;
- GPU names, vendors, counts, VRAM, VRAM source/confidence, backend, and
  unified-memory status;
- total GPU count and VRAM;
- detected backend;
- CUDA, ROCm, and Vulkan availability;
- NVIDIA and AMD driver versions when available.

The payload intentionally excludes:

- timestamps;
- available RAM;
- agent and benchmark versions;
- container/VM runtime flags;
- provider identity and `machine_id`;
- API tokens, private keys, credentials, paths, and other secrets.

Excluding provider identity keeps the technical fingerprint stable across
identity initialization, migration, and signing-key rotation.

## Propagation

Newly generated values appear in:

- `burd-agent fingerprint --json`;
- full and signed reports;
- provider details;
- provider verification;
- registration payloads;
- challenge responses.

New challenge responses sign the hardware fingerprint together with the
challenge ID, nonce, provider ID, machine ID, and report hash. Existing legacy
responses without a fingerprint can still be parsed, but they do not satisfy
new fingerprint-bound verification when the signed report contains one.

Provider verification compares the current fingerprint with the latest signed
report and challenge response. A mismatch records a failed verification check,
raises fraud risk, removes local self-verification, and causes readiness to
surface the hardware change.

## Invalidating Changes

The fingerprint changes when relevant technical evidence changes, including:

- GPU model or count;
- VRAM capacity, source, or confidence;
- backend;
- CUDA/ROCm/Vulkan availability;
- critical driver version;
- CPU, core count, total RAM, OS, or architecture.

Runtime-only values such as timestamps, available RAM, and container flags do
not change the fingerprint.
