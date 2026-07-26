# BN-03 - Remote Session And Control Channel

BN-03 makes the Burd control plane authoritative for provider connectivity.
The local `session` command remains a local evidence snapshot; the new
`remote-session` command maintains the authenticated network session.

## Implemented Flow

1. An enrolled agent sends `POST /v1/sessions` with its short-lived device
   credential, backend provider/device IDs, enrollment-bound hardware
   fingerprint, agent version, and capability snapshot.
2. The backend creates one nonterminal session per device and returns a resume
   token once. PostgreSQL stores only its SHA-256 hash.
3. The agent opens the returned WebSocket URL as an outbound connection using
   the device credential, resume token, and device ID in headers.
4. The backend marks the session `online` only after the channel is connected.
5. Heartbeats carry a monotonic sequence and fingerprint. The backend records
   server receipt time, payload hash, sequence gap, status, and rolling TTL.
6. Socket loss or missed heartbeats marks the session `offline`; sequence gaps
   or fingerprint mismatch mark it `degraded`.
7. The agent reconnects with bounded exponential backoff and per-agent
   jitter, and resumes the same session. Hardware registration is collected
   during blocking connection preparation; the connection reuses the
   enrollment-bound fingerprint for heartbeats and signed telemetry instead of
   repeating hardware detection on every interval.
8. Administrative revocation updates PostgreSQL and signals the active socket.

## States

```text
pending_connection
-> online
-> degraded | offline
-> online
-> expired | revoked
```

`expired` and `revoked` are terminal. `online`, `offline`, and `degraded` are
backend-derived; the agent cannot set them.

## Authentication

- `Authorization: Bearer <device credential>`
- `X-Burd-Session-Token: <resume token>`
- `X-Burd-Device-Id: <device ID>`

Secrets are not accepted in URLs, query strings, telemetry payloads, status
responses, or audit metadata. Device credentials and resume tokens are stored
as hashes by the backend.

## Commands

```powershell
burd-agent remote-session connect
burd-agent remote-session connect --max-reconnect-delay-seconds 60
burd-agent remote-session status --json
```

`connect` runs until interrupted. It refreshes the short-lived device
credential before expiry and persists only the data needed for process restart
and session resume in `~/.burd/remote-session.json`. Transient transport, HTTP
`408`/`429`/`5xx`, and conflict failures retry with a bounded exponential
ceiling and per-agent jitter. The failure counter resets only after a heartbeat
acknowledgement proves the connection is usable. Revoked or invalid credentials stop
the command; missing or expired sessions are recreated. Malformed or unreadable
local session state stops instead of being treated as absent. Resume is allowed
only for the Control Plane recorded by the current enrollment, and successful
re-enrollment invalidates the previous local session. Retry attempts are in-memory
and reset when the process restarts. An ignored PostgreSQL integration test runs the
real Agent loop through enrollment, heartbeat acknowledgement, socket loss, resume,
server-side expiry, replacement session creation, and administrative revocation.

## Configuration

- `BURD_CONTROL_SESSION_TTL_SECONDS` defaults to `900`.
- `BURD_CONTROL_HEARTBEAT_INTERVAL_SECONDS` defaults to `15`.
- `BURD_CONTROL_MISSED_HEARTBEAT_LIMIT` defaults to `3`.

The backend periodically expires stale sessions independently of requests.

## Deferred

BN-03 itself introduced heartbeat messages on the control channel. BN-04 extends
that channel with signed telemetry. A supervised agent daemon, durable retry
attempt state, backend challenge execution, jobs, and scheduling remain outside
this hardening scope. The lifecycle and service-packaging gates are frozen in
[`hardening/agent-service-lifecycle-contract.md`](hardening/agent-service-lifecycle-contract.md).
