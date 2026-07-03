# Local API

Run:

```sh
burd-agent serve --host 127.0.0.1 --port 8787
```

Stop with `Ctrl+C`.

Use `127.0.0.1` for the MVP. Running on `0.0.0.0` exposes the local API beyond
loopback and emits a strong warning if no API token is configured.

Create a token:

```sh
burd-agent api-token create --json
```

The token is printed once. Later `api-token show --json` only shows status. Send
the token to protected endpoints with:

```sh
Authorization: Bearer <token>
```

When `api_auth_enabled` is true, protected endpoints reject missing or invalid
tokens with HTTP 401.

Endpoints:

- `GET /health`
- `GET /api/v1/system`
- `GET /api/v1/fit`
- `GET /api/v1/score`
- `GET /api/v1/report`
- `POST /api/v1/report/signed`
- `GET /api/v1/challenge/mock`
- `GET /api/v1/provider`
- `GET /api/v1/verification`
- `GET /api/v1/uptime`
- `GET /api/v1/reliability`
- `GET /api/v1/network-score`
- `GET /api/v1/trust-score`
- `GET /api/v1/capability-spot`
- `GET /api/v1/workload-eligibility`
- `GET /api/v1/history`
- `GET /api/v1/registration-payload`
- `GET /api/v1/pricing`
- `GET /api/v1/earnings`
- `GET /api/v1/actions`
- `GET /api/v1/logs`
- `GET /api/v1/raw`
- `GET /api/v1/config`
- `POST /api/v1/benchmark/run`
- `POST /api/v1/challenge/run`
- `GET /api/v1/benchmark/status`

The UI is served at `/`.

Notes:

- `POST /api/v1/benchmark/run` may execute real local benchmarks.
- `POST /api/v1/challenge/run` requires identity and a private key.
- Raw data is redacted and must not include private key material.

Protected endpoints:

- `GET /api/v1/raw`
- `GET /api/v1/config`
- `GET /api/v1/report`
- `POST /api/v1/report/signed`
- `POST /api/v1/benchmark/run`
- `POST /api/v1/challenge/run`

Public or lower-risk endpoints:

- `GET /health`
- read-only summaries such as system, provider, verification, history, uptime,
  reliability, network score, trust score, capability spot, workload eligibility,
  pricing, earnings, actions, and logs.
