# BN-07 - Recurring And Risk-Based Verification

BN-07 turns the BN-06 Proof of Capability protocol into backend-owned verification state. It does not add the agent-side CUDA/VRAM/GEMM/LLM workload runner, global trust scoring, antifraud ranking, scheduler, jobs, marketplace, billing, Pix, or payouts.

## Scope

Implemented:

- PostgreSQL `provider_verification_states` keyed by `(provider_id, device_id)`;
- verification statuses for `new_provider`, `verified`, `verification_due`, `verification_running`, `suspect`, `quarantined`, and `blocked` policy states;
- policy metadata on `proof_challenges`: trigger reason, risk reasons, and verification policy version;
- admin-triggered verification sweep that evaluates online/degraded remote sessions and issues BN-06 challenges when due;
- retry budget, periodic due interval, sweep limit, and suspect-failure threshold from config;
- challenge expiry handling during sweeps, with expired running verifications converted into failed verification state;
- automatic state transitions when a BN-06 proof response verifies or fails;
- audit events for challenge issuance by policy, verified state, and failed state;
- admin API to list verification states for provider devices.

Not implemented:

- background scheduler process for automatic sweep cadence;
- agent-side active proof workload execution;
- risk model using job history, regional probes, duplicate GPU detection, or performance history;
- trust/antifraud score publication;
- scheduler enforcement, leases, paid jobs, billing, Pix, payouts, or marketplace listings.

## State Model

The verification policy state is per provider device, not per human account:

```text
new_provider
-> verification_due
-> verification_running
-> verified
-> verification_due
-> verification_running
-> suspect | verified
-> quarantined | blocked
```

BN-07 currently writes:

- `verification_running` when a sweep issues a challenge;
- `verified` when the BN-06 verifier accepts a signed proof response;
- `verification_due` when a proof fails but the failure threshold has not been reached;
- `suspect` when repeated failures meet `BURD_CONTROL_VERIFICATION_SUSPECT_FAILURES`.

`quarantined` and `blocked` remain reserved for later policy/admin decisions. The sweep will not issue new challenges for those states.

## API

### `POST /v1/verification/sweep`

Admin endpoint. Runs one bounded verification sweep.

Request fields:

- `limit`, optional, capped by `BURD_CONTROL_VERIFICATION_SWEEP_LIMIT`;
- `force`, optional, defaults to `false`;
- `reason`, optional short printable ASCII reason.

The sweep:

- expires stale issued/acknowledged/running proof challenges by server time;
- updates verification state for expired running challenges;
- evaluates remote sessions in `online` or `degraded` state;
- skips blocked/quarantined providers and inactive devices;
- skips sessions with an active nonterminal challenge;
- issues a BN-06 proof challenge for new, due, forced, suspect, or stale-running verification states;
- uses latest accepted GPU telemetry UUID when available;
- binds the challenge to the backend session hardware fingerprint.

Returns:

- `request_id`;
- `evaluated`, the number of candidate sessions inspected;
- `issued`, the provider/device/session/challenge IDs created by the sweep.

### `GET /v1/providers/{provider_id}/verification-states`

Admin endpoint. Lists backend verification state rows for provider devices.

Each row includes:

- provider and device IDs;
- status and policy version;
- reason;
- risk score;
- success/failure counts;
- retry budget remaining;
- last challenge and last verified challenge;
- last verified/failed timestamps;
- next due timestamp;
- quarantine/block timestamps when later policy sets them.

## Config

- `BURD_CONTROL_VERIFICATION_PERIOD_SECONDS`, default `3600`.
- `BURD_CONTROL_VERIFICATION_RETRY_BUDGET`, default `2`.
- `BURD_CONTROL_VERIFICATION_SWEEP_LIMIT`, default `25`.
- `BURD_CONTROL_VERIFICATION_SUSPECT_FAILURES`, default `3`.

## Server Authority

The backend remains authoritative for recurrence. Providers do not decide whether they are verified, due, suspect, quarantined, blocked, or trusted. Provider-sent score, expiry, eligibility, online, or local capability flags are not used as authority.

BN-07 intentionally keeps the sweep as an admin-triggered operation. A future scheduler can call the same API or internal method without changing the verification state contract.