# BN-09 - Global Trust And Antifraud

BN-09 turns backend-owned signals from BN-02 through BN-08 into a first global trust and antifraud state. The provider does not submit its own global reputation, final trust score, risk score, or antifraud status.

## Implemented Scope

- `burd-protocol` defines trust sweep, provider trust state, and antifraud event response contracts.
- PostgreSQL migration `0009_global_trust_antifraud` adds `provider_trust_states` and `antifraud_events`.
- The control plane exposes admin endpoints to run a bounded trust sweep, list trust states, and list antifraud events.
- Trust state is recalculated from backend-observed enrollment/device state, latest remote session, sequenced heartbeats, telemetry presence, remote evidence count, proof challenge history, recurring verification state, and remote network state.
- Antifraud events are recorded for suspicious backend-observed conditions such as heartbeat without telemetry, degraded sessions, repeated challenge failures, weak remote network, duplicate GPU UUIDs, fingerprint reuse, suspect verification, and missing evidence/proof during cold start.
- Every recalculated trust state emits an audit event.

## API

### `POST /v1/trust/sweep`

Admin endpoint. Runs one bounded global trust and antifraud pass.

Request fields:

- `limit`, optional and capped by backend policy;
- `force`, reserved for later policy behavior;
- `reason`, optional short printable ASCII reason.

Response fields:

- `request_id`
- `evaluated`
- `updated[]` with `provider_id`, `device_id`, `status`, `trust_score`, `risk_score`, and `reason_codes`.

### `GET /v1/providers/{provider_id}/trust-states`

Admin endpoint. Lists backend-calculated trust states for the provider's devices.

State fields include:

- `status`
- `policy_version`
- `trust_score`
- `risk_score`
- optional `reliability_score`
- optional `verification_status`
- optional `remote_network_score`
- evidence and challenge counts
- latest session status
- latest GPU UUID and hardware fingerprint
- `reason_codes`
- creation/update timestamps

### `GET /v1/providers/{provider_id}/antifraud-events`

Admin endpoint. Lists recent backend antifraud events for a provider. The optional `limit` query parameter is clamped to `1..200`.

Event fields include:

- `event_id`
- `provider_id`
- `device_id`
- `event_type`
- `severity`
- `status`
- `reason`
- redacted `metadata`
- first/last seen timestamps
- `occurrence_count`

## Trust States

BN-09 can produce these trust statuses:

```text
new_provider
insufficient_history
trusted
highly_trusted
degraded
suspect
blocked
```

Cold start is handled separately. A new provider without evidence, successful proof challenges, or telemetry can remain `new_provider` instead of being treated as fraudulent. Sustained risk, duplicate identity signals, repeated failures, blocked provider/device state, or inconsistent backend observations lower trust and raise risk.

## Authority Rules

- Providers do not choose their trust score, risk score, antifraud state, online state, verification state, or remote network score.
- Server-side session, telemetry, evidence, challenge, verification, and probe history are the scoring inputs.
- Provider-sent local trust, local reliability, local network, local eligibility, and local expiry flags remain non-authoritative.
- Audit history records recalculation decisions, but BN-09 does not create marketplace ranking or paid workload routing.

## Not Implemented Yet

- Production machine-learning or rules-engine antifraud model.
- Admin resolution workflow for antifraud cases.
- Automatic quarantine/block transitions from trust sweep output.
- Scheduler consumption of trust state.
- Marketplace ranking, listings, reservations, jobs, leases, billing, Pix, payouts, or disputes.
