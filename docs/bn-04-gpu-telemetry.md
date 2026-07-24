# BN-04 - Signed GPU Telemetry

BN-04 turns NVIDIA GPU measurements into signed, session-bound observations
that the Burd control plane can verify and retain.

## Collection

The first collector uses NVIDIA's structured `nvidia-smi` selective queries
with `csv,noheader,nounits`. Queries are split into capability groups so one
unsupported metric does not discard the required GPU identity.

Required:

- GPU UUID;
- GPU name;
- PCI bus and vendor/device IDs when available;
- NVIDIA driver;
- total VRAM.

Collected when supported:

- compute capability and CUDA driver compatibility version;
- used/free VRAM;
- GPU and memory utilization;
- temperature;
- power draw and power limit;
- graphics, SM, and memory clocks;
- performance state and active throttle reasons;
- volatile corrected/uncorrected ECC totals;
- compute process PID, basename, and GPU memory.

CUDA runtime version, container ID, and Burd job ID remain nullable until a
runtime/job context can attest them. Process command lines and full paths are
never collected.

NVIDIA recommends UUID or PCI bus ID for stable selection because enumeration
order can change across reboots:

- <https://docs.nvidia.com/deploy/nvidia-smi/index.html>
- <https://docs.nvidia.com/deploy/nvml-api/index.html>

NVML or DCGM can replace the collector later without changing the signed Burd
telemetry contract.

## Signed Batch

Every batch contains:

- schema version `burd-gpu-telemetry-v1`;
- provider, device, and session IDs;
- control-channel sequence;
- contiguous sample sequence range;
- enrollment-bound hardware fingerprint;
- collector version;
- collection window;
- GPU samples;
- canonical payload hash;
- active public-key ID;
- Ed25519 signature.

The signature uses the `burd.telemetry-batch.v1` domain and binds the batch
hash, authority IDs, both sequence domains, fingerprint, and public-key ID.

## Backend Verification

The backend:

1. authenticates the device credential and session resume token;
2. requires an online or degraded session;
3. validates schema, sample count, timestamps, ranges, and redaction;
4. checks control and sample sequence continuity;
5. recalculates the canonical batch hash;
6. loads the active device public key and verifies Ed25519;
7. persists the batch and normalized samples transactionally;
8. returns `telemetry_ack`;
9. applies retention using server receipt time.

Telemetry messages do not replace heartbeats and do not renew the missed
heartbeat deadline.

## Agent

Telemetry remains explicit:

```powershell
burd-agent remote-session connect --telemetry
burd-agent remote-session connect --telemetry --telemetry-batch-samples 8
```

It is also enabled when `telemetry_enabled` is true in the agent identity
configuration. Without `nvidia-smi`, the remote session remains connected and
reports telemetry as unavailable in structured stderr logs.

## Configuration

- `BURD_CONTROL_TELEMETRY_MAX_SAMPLES_PER_BATCH=64`
- `BURD_CONTROL_TELEMETRY_MIN_BATCH_INTERVAL_SECONDS=5`
- `BURD_CONTROL_TELEMETRY_CLOCK_SKEW_SECONDS=300`
- `BURD_CONTROL_TELEMETRY_RETENTION_DAYS=7`

## Deferred

DCGM cluster metrics, runtime-attested container/job association, remote
network probes, agent-orchestrated challenge telemetry capture, and
trust/antifraud scoring remain deferred to later BN phases. BN-06 proof
verification can already require a `telemetry_window_hash` to reference a
server-accepted BN-04 batch for the same session and GPU.
