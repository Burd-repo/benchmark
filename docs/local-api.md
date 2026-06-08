# Local API

Run:

```sh
burd-agent serve --host 127.0.0.1 --port 8787
```

Stop with `Ctrl+C`.

Use `127.0.0.1` for the MVP. Running on `0.0.0.0` exposes the local API beyond
loopback and only emits a warning today. API token support is future work.

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
- `GET /api/v1/pricing`
- `GET /api/v1/earnings`
- `GET /api/v1/actions`
- `GET /api/v1/logs`
- `GET /api/v1/raw`
- `POST /api/v1/benchmark/run`
- `POST /api/v1/challenge/run`
- `GET /api/v1/benchmark/status`

The UI is served at `/`.

Notes:

- `POST /api/v1/benchmark/run` may execute real local benchmarks.
- `POST /api/v1/challenge/run` requires identity and a private key.
- Raw data is redacted and must not include private key material.

