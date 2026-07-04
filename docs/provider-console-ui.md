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
- `GET /api/v1/readiness`
- `GET /api/v1/verification`
- `GET /api/v1/uptime`
- `GET /api/v1/reliability`
- `GET /api/v1/network-score`
- `GET /api/v1/ai-performance`
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
- `GET /api/v1/benchmark/status`

If local API auth is enabled, protected endpoints can return `401` until the
request sends `Authorization: Bearer <token>`. The UI keeps the failure visible
instead of hiding it.

## Console Sections

The UI includes:

1. Overview
2. Readiness
3. Hardware
4. Benchmarks
5. Workloads
6. History
7. Uptime
8. Security
9. Registration
10. Logs
11. Raw Data

Overview shows provider and machine identity, local online status, Compute Score, local readiness, GPU, backend, suggested demonstrative price, local reliability, local network assessment, local heuristic trust, local/mock capability, future marketplace status, session status, signed report status, and challenge status.

Readiness shows the canonical local readiness score, level, status, individual checks, warnings, recommendations, and evidence state from `GET /api/v1/readiness` plus local verification evidence.

Hardware shows CPU, RAM, GPU, VRAM, backend, driver/runtime signals, disk, hardware fingerprint summary, VRAM source/confidence, marketplace GPU policy, and fingerprint mismatch warnings.

Benchmarks shows consolidated AI Performance from `GET /api/v1/ai-performance`, including measured/estimated/expired/not-measured status, source, confidence, model/runtime/backend, tokens per second, TTFT, latency where measured, compatible models, network score, latency, jitter, loss, DNS timing, disk, and fit analysis.

Workloads shows local workload eligibility and future marketplace status from `GET /api/v1/workload-eligibility`, including reasons and blockers. It never labels a workload as approved.

History shows the latest benchmark and prior persisted benchmark summaries when they exist, including score, tier, signed state, and challenge ID.

Uptime shows local heartbeat/session state, local online status, reliability, uptime ratios, fingerprint match state, and no-history warnings without treating missing samples as fraud.

Security shows public key summary, signature status, API token status, challenge verification, fraud risk, and raw-data redaction status.

Registration shows the local registration payload for future backend use, a copy button, and a disabled future backend send action.

Logs shows local actions and task logs.

Raw Data shows formatted redacted JSON. Private keys, API tokens, token hashes, and credential fields must never be displayed.
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

## Provider Console Integration - PR 11

The console now consumes the dedicated PR 7-10 endpoints directly before falling back to provider aggregates or raw data:

- AI Performance is shown inside `Benchmarks` from `GET /api/v1/ai-performance`.
- Local Trust Assessment is summarized in `Overview` from `GET /api/v1/trust-score`.
- Capability Spot - Local/Mock is summarized in `Overview` from `GET /api/v1/capability-spot`.
- Workloads is a dedicated tab backed by `GET /api/v1/workload-eligibility`.

The UI labels local-only, local-heuristic, local/mock, future-marketplace, token-required, not-measured, unavailable, expired, and missing states explicitly. It does not calculate scores in the browser; it renders backend contracts and source labels. The benchmark action remains visible but asks for confirmation because it can be a heavy manual local operation.

There is no API-token input in this phase. Protected endpoints that return `401` or `403` show `Token required` while the rest of the console remains usable.