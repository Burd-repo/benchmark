# Heartbeat

Heartbeat is the one-shot local liveness check for an active provider session.
It does not create a daemon, loop continuously, contact a backend, or imply
marketplace admission.

Run:

```sh
burd-agent heartbeat --once --json
```

## Contract

The heartbeat payload reports the current local snapshot, including:

- timestamp
- provider and machine ids
- provider session id
- session status
- hardware fingerprint
- fingerprint/session match flag
- GPU name, vendor, count, VRAM total/source/confidence, backend, and CUDA/
  ROCm/Vulkan availability when known
- local online flag
- optional utilization fields for GPU load, VRAM used/free, CPU load, and
  available memory
- heartbeat count
- heartbeat summary
- health snapshot
- warnings

Unavailable utilization values are returned as `null`; they are not invented.

## Behavior

- Requires an existing provider session.
- Requires the session to be active and not expired.
- Invalidates the session when the current hardware fingerprint no longer
  matches the fingerprint stored in the session.
- Updates `last_heartbeat_at`, `heartbeat_count`, and the local uptime history
  when the heartbeat succeeds.
- Returns a stable JSON contract without exposing secret key material or API
  tokens.

## Persistence

The heartbeat history is stored locally in `~/.burd/uptime.json` unless
`BURD_AGENT_HOME` or `BURD_AGENT_CONFIG` redirects the canonical state
directory. Tests use temporary state directories and never touch the real home
folder.

## Contract Notes

- Heartbeat once is not a backend availability signal.
- Heartbeat once is not a marketplace readiness decision.
- Heartbeat once prepares the local data that a later reliability score could
  consume.
- Provider details, raw data, registration payloads, and readiness may surface
  the latest heartbeat summary when one exists.
