# Provider Console UI

The Provider Console UI lives in `apps/benchmark-ui` and is embedded into the
local API binary through `include_str!`. It is served at `/` by
`burd-agent serve`.

This is a local provider operations console. It is not the Burd institutional
landing page and must keep following the dark technical design system in
`SKILL.md`.

## Build

From Windows PowerShell at the repository root:

```powershell
cargo build
```

## Start The Local API

```powershell
.\target\debug\burd-agent.exe serve --host 127.0.0.1 --port 8787
```

Open:

```txt
http://127.0.0.1:8787/
```

Stop the foreground server with `Ctrl+C`.

If a development process is stuck, stop it explicitly:

```powershell
Get-Process burd-agent -ErrorAction SilentlyContinue | Stop-Process -Force
```

## Test Without Hanging

Use the API smoke script:

```powershell
.\scripts\test-api.ps1
```

The script:

- starts `.\target\debug\burd-agent.exe serve --host 127.0.0.1 --port 8787`;
- waits up to 5 seconds for `/health`;
- calls the main GET endpoints;
- writes output to `tmp/test-output/`;
- stops the server by PID in a `finally` block.

Use the local CLI smoke script:

```powershell
.\scripts\test-local.ps1
```

It runs formatting, tests, build, and fast local commands. It does not start
`serve`, does not run `heartbeat --interval`, and does not run heavy benchmarks.

## API Data Used By The UI

The UI consumes these local endpoints when opened through `serve`:

- `GET /health`
- `GET /api/v1/system`
- `GET /api/v1/fit`
- `GET /api/v1/score`
- `GET /api/v1/report`
- `GET /api/v1/provider`
- `GET /api/v1/verification`
- `GET /api/v1/uptime`
- `GET /api/v1/history`
- `GET /api/v1/registration-payload`
- `GET /api/v1/pricing`
- `GET /api/v1/earnings`
- `GET /api/v1/actions`
- `GET /api/v1/logs`
- `GET /api/v1/raw`
- `GET /api/v1/config`
- `GET /api/v1/benchmark/status`

If local API auth is enabled, protected endpoints can return `401` until the
request sends `Authorization: Bearer <token>`. The UI keeps the failure visible
instead of hiding it.

## Console Sections

The UI includes:

1. Overview
2. Hardware
3. Benchmarks
4. History
5. Uptime
6. Security
7. Registration
8. Logs
9. Raw Data

Overview shows provider and machine identity, online status, Burd Compute Score,
tier, local marketplace readiness, GPU, backend, suggested demonstrative price,
uptime, signed report status, and challenge status.

Hardware shows CPU, RAM, GPU, VRAM, backend, driver/runtime signals, disk, and
network summary.

Benchmarks shows LLM, stability, network, disk, fit analysis, and skipped,
passed, or failed status.

History shows the latest benchmark and prior persisted benchmark summaries when
they exist, including score, tier, signed state, and challenge ID.

Security shows public key summary, signature status, API token status, challenge
verification, fraud risk, and raw-data redaction status.

Registration shows the local registration payload for future backend use, a copy
button, and a disabled future backend send action.

Logs shows local actions and task logs.

Raw Data shows formatted redacted JSON. Private keys, API tokens, token hashes,
and credential fields must never be displayed.

## Offline API Message

If the UI cannot reach the local API, it shows:

```txt
API local da Burd nao esta rodando. Execute:
.\target\debug\burd-agent.exe serve --host 127.0.0.1 --port 8787
```

This replaces unhelpful browser errors such as `Failed to fetch`.

## Still Local Or Mocked

The console intentionally does not implement a production backend, Pix, billing,
payouts, real marketplace listings, real jobs, leases, orchestration, reputation
ranking, TPM/HSM attestation, remote storage, or remote telemetry.

