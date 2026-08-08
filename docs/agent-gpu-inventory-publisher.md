# Agent Signed GPU Inventory Publisher

The Agent now owns publication of the complete local NVIDIA GPU inventory. The worker is
supervised by the existing remote-session lifecycle and runs independently of runtime observation,
runtime verification and the deliberately disabled production Provider Job Worker.

## Lifecycle

The publisher starts with `remote-session connect`, waits fail-closed until a persisted session is
available and then publishes immediately. It watches the local provider/device/session/key binding
every two seconds and probes hardware every 60 seconds.

Publication is required when:

- the remote session changes;
- the active device key changes after rotation or re-enrollment;
- the hardware fingerprint changes;
- a GPU is added, removed or materially changes identity;
- the worker restarts and has no successful in-memory publication state.

Transient local-state, discovery, HTTP and Control Plane failures do not terminate the remote
session. The worker retries with bounded exponential backoff. While no valid snapshot has been
accepted, runtime admission remains denied by the Control Plane.

## Discovery and snapshot semantics

The publisher uses the local NVIDIA discovery shared with telemetry, not runtime observation or
Control Plane input. The required `nvidia-smi` identity query supplies physical GPU index, UUID,
PCI vendor/device IDs and total VRAM.

One `SignedDeviceGpuInventory` contains every detected GPU. Devices are sorted by physical index
and UUID before canonical hashing. Duplicate indices, case-insensitive duplicate UUIDs, malformed
UUIDs, missing PCI identity, zero VRAM or an empty snapshot fail the entire publication; partial
snapshots are never sent.

This version publishes `status=active` only for fully identified NVIDIA CUDA devices. An
unidentifiable device causes discovery failure rather than being silently advertised as active.
GPU absence is represented by omission from the next complete snapshot. Control Plane eligibility
queries use only that latest complete snapshot, so an omitted historical GPU is no longer current
supply.

## Signing and deduplication

The payload binds provider, device, session, current hardware fingerprint, observation time and the
complete GPU list. The Agent computes the canonical inventory hash and signs the protocol message
with the current Ed25519 device key.

The worker also calculates a local publication fingerprint that excludes only `observed_at` and
includes session, key, hardware and the sorted GPU list. An unchanged periodic probe is not sent
again. Failed submissions retain the exact signed envelope for retry, allowing the Control Plane's
inventory-hash deduplication to handle a lost response without creating a new snapshot.

Immediately before HTTP submission, the Agent reloads enrollment and session state. Any provider,
device, session, key or Control Plane URL change discards the prepared envelope and rebuilds it
under the current binding.

## Transport and logging

The publisher submits to `POST /v1/sessions/{session_id}/gpu-inventory` with the existing
device/session authentication headers. Credentials, resume tokens, private keys and signatures are
never included in publisher logs. Logs contain only stable event/reason codes and, after success,
the non-secret inventory hash.

## Deferred work

- scheduler consumption of runtime admission;
- assignment-time admission revalidation;
- production Provider Job Worker activation;
- non-NVIDIA inventory backends;
- persistent local publication state across Agent restarts.
