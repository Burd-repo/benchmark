# Provider Session

Provider Session is the local, expirational state that means:

> This provider is trying to be available on the network now.

It is intentionally local-only in this PR. There is no backend session registry,
no heartbeat loop, and no real marketplace admission.

## Commands

```sh
burd-agent session start --json
burd-agent session status --json
burd-agent session stop --json
```

## Session Model

The local session snapshot stores:

- `provider_session_id`
- `provider_id`
- `machine_id`
- `hardware_fingerprint`
- `started_at`
- `last_heartbeat_at`
- `status`
- `readiness_at_start`
- `report_hash`
- `challenge_id`
- `expires_at`
- `marketplace_policy_snapshot`
- `evidence_summary`
- `session_mode`
- `online_locally`
- `is_expired`
- `warnings`

When a heartbeat snapshot exists, the session can also carry:

- `heartbeat_count`
- `last_heartbeat_status`
- `last_heartbeat_error`
- `last_heartbeat_fingerprint_matches_session`
- `last_heartbeat_warnings`

The status set is:

- `inactive`
- `active`
- `expired`
- `invalidated`
- `stopped`
- `failed`

## Behavior

- `session start` requires local identity, valid signed report evidence,
  valid challenge evidence, matching hardware fingerprint, and readiness that
  resolves to `ready_locally`.
- Supported NVIDIA/CUDA hardware starts a marketplace-local session mode.
- Unsupported hardware can still start a local diagnostic session mode when
  local readiness is valid, but it is not promoted to marketplace eligibility.
- `session status` re-evaluates expiry and invalidation against the latest
  evidence.
- `session stop` marks the session as stopped and offline locally.
- `heartbeat --once --json` updates `last_heartbeat_at`, appends local uptime
  history, and records a one-shot local liveness snapshot when the session is
  active.
- A fingerprint mismatch invalidates the local session and prevents the
  heartbeat from being treated as online.

## Persistence

The session is stored locally at `~/.burd/provider-session.json` unless
`BURD_AGENT_HOME` or `BURD_AGENT_CONFIG` redirects state to another directory.
Tests use temporary state directories and never touch the real home folder.

## Contract Notes

- The session snapshot is redacted and contains no secret key material.
- `readiness`, `provider`, `raw`, and `registration` payloads can include a
  session summary when one exists.
- `provider`, `raw`, `registration`, and readiness may also surface the most
  recent heartbeat summary when one exists, but that remains a local-only
  snapshot.
- This is a local contract only. Backend sessions, session leases, heartbeat
  loops, and marketplace admission remain future work.

## Relationship To Workload Eligibility

An active local session is evidence of intended availability, not a lease. Future marketplace and scheduler policy should require active session state and recent heartbeat before routing online workloads, but the local MVP only records and reports the state.