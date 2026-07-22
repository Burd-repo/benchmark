# BN-21 - Multi-GPU Inventory Foundation

BN-21 adds the first backend-owned multi-GPU inventory registry. It makes the control plane authoritative for which GPU UUIDs belong to a provider device, which snapshot was observed, and which jobs or scheduler decisions may rely on that inventory.

## Scope

Implemented in this stage:

- protocol structs for a signed device GPU inventory snapshot;
- canonical inventory hash and Ed25519 signature message binding provider, device, session, hardware fingerprint, public key, and inventory hash;
- append-only PostgreSQL `device_gpu_inventory` registry with immutable per-GPU rows;
- backend verification of inventory hash, active key, session binding, and hardware fingerprint binding;
- admin endpoint for listing provider GPU inventory history;
- device endpoint for submitting signed GPU inventory through an authenticated remote session;
- job and scheduler checks that require an active inventory row for the requested GPU UUID;
- audit events for accepted and rejected inventory submissions.

## API

### `POST /v1/sessions/{session_id}/gpu-inventory`

Device endpoint. Requires the short-lived device bearer credential for the remote session. The submitted payload is signed by an active provider device key.

The backend accepts the inventory only when:

- `schema_version` and canonicalization version are supported;
- provider, device, and session IDs match the authenticated session;
- the session is currently `online` or `degraded`;
- the submitted hardware fingerprint matches the remote session fingerprint;
- `inventory_hash` equals the canonical payload hash;
- `public_key_id` is active for that provider device;
- the Ed25519 signature verifies against the inventory signature message.

A valid inventory snapshot is deduplicated by `(inventory_hash, gpu_index)` when the same hash is resubmitted. Invalid signature, inactive key, bad binding, or bad hash is rejected and audited.

### `GET /v1/providers/{provider_id}/gpu-inventory`

Admin endpoint. Lists immutable GPU inventory records for a provider. The response includes per-GPU UUID, GPU index, backend, PCI IDs, VRAM, status, and backend verification state.

## Authority Rules

The agent may claim GPU UUIDs, GPU indices, backend labels, PCI IDs, VRAM totals, and per-GPU status, but those fields are not final inventory truth. The backend only attests that the inventory was signed by the active device key, bound to the remote session and hardware fingerprint, and stored as an immutable snapshot.

`server_received_at` is authoritative for registry ordering. Provider timestamps are recorded as observations only.

Jobs and scheduler leases must check the latest stored inventory row for the requested `gpu_uuid`; older active rows do not make a GPU eligible after a newer inactive, degraded, or retired snapshot.

## Non-Goals

BN-21 does not implement:

- distributed multi-provider placement;
- GPU-level resource reservation across multiple providers;
- Kubernetes device plugins or cluster orchestration;
- performance policy changes beyond inventory presence checks;
- runtime isolation changes from BN-12.

These remain follow-up work after the authoritative inventory boundary is stable.
