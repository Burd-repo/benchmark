# BN-08 - Regional Network Probes

BN-08 replaces provider-owned network truth with backend-owned observations from trusted regional probes. It does not deploy a global probe fleet, scheduler, marketplace ranking, jobs, billing, Pix, or payouts.

## Implemented Scope

- `burd-protocol` defines regional network probe observation, observation history, regional reachability, and provider network state response contracts.
- PostgreSQL migration `0008_regional_network_probes` adds immutable-ish probe observation history and per-provider-device network state.
- The control plane exposes admin/probe endpoints to submit observations, list probe history, and read backend-calculated network state.
- The backend validates metric ranges, timestamps, redacted metadata, provider/device/session binding, active device state, and nonblocked provider state.
- Duplicate observations are deduplicated by `(session_id, probe_id, observed_at)` and returned with `duplicate=true`.
- `remote_network_score`, `regional_reachability`, and `effective_network_score` are calculated by the backend, never accepted from the provider.
- Audit events are emitted for accepted observations.

## API

### `POST /v1/network-probes/observations`

Admin/probe endpoint. Submits a trusted regional observation for an existing remote session.

Request fields:

- `provider_id`
- `device_id`
- `session_id`
- `probe_id`
- `probe_region`
- `observed_at`
- `sample_count`
- optional `control_rtt_ms`
- optional `jitter_ms`
- optional `packet_loss_percent`
- optional `reconnect_count`
- optional `upload_mbps`
- optional `download_mbps`
- optional `artifact_throughput_mbps`
- optional `stability_score`
- optional `approximate_region`
- optional `path_consistency`
- optional redacted `metadata`

Response fields include:

- `duplicate`
- stored observation metadata
- backend-calculated `remote_network_score`
- observation `status`: `reachable`, `degraded`, or `unreachable`
- recalculated provider `network_state`

### `GET /v1/providers/{provider_id}/network-probes`

Admin endpoint. Lists recent probe observations for a provider. The optional `limit` query parameter is clamped to `1..200`.

### `GET /v1/providers/{provider_id}/network-state`

Admin endpoint. Lists backend-owned network state rows for provider devices.

State fields:

- `local_network_score`: reserved nullable field for future blending with accepted local benchmark evidence;
- `remote_network_score`: backend aggregate over recent trusted probe observations;
- `regional_reachability`: latest observation per probe region;
- `effective_network_score`: currently remote score when no accepted local score exists;
- `sample_count`, `last_observed_at`, and `updated_at`.

## Authority Rules

- Providers do not submit remote network scores.
- Provider-declared region remains a claim, not proof.
- Local `bench network` remains useful local evidence, but it is not remote reachability.
- BN-08 measures the existing outbound remote session and future data-plane paths; it does not require providers to open inbound public ports.
- Server receipt time and trusted probe observations are authoritative for the remote score.

## Not Implemented Yet

- Deployed multi-region probe workers.
- Probe-specific credentials separate from the bootstrap admin bearer.
- Artifact upload/download data-plane probes against real job artifacts.
- Scheduler use of network state.
- Global trust/antifraud scoring from network history.
- Marketplace region filtering, ranking, pricing, jobs, leases, billing, Pix, or payouts.