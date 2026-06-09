# Burd Benchmark

Burd Benchmark is the local validation product for the Burd GPU marketplace. It
installs as `burd-agent`, detects provider hardware, estimates which AI workloads
fit, runs real local inference benchmarks when runtimes are available, calculates
a Burd Compute Score, generates signed reports, tracks local uptime/actions/logs,
and serves a Provider Console base for future marketplace validation.

This is not the Burd institutional landing page.

## Relationship with llmfit

This repository uses `llmfit` as the technical foundation instead of rewriting
hardware and model-fit logic from scratch. The current strategy is an adapter
integration: Burd crates depend on `third_party/llmfit/llmfit-core` and layer
Burd-specific reporting, scoring, protocol, and API behavior on top.

Credits and license details are documented in `NOTICE.md` and
`docs/llmfit-adaptation.md`.

## Build

```sh
cargo build
```

On Windows PowerShell, use the debug binary directly until the agent is
installed on `PATH`:

```powershell
.\target\debug\burd-agent.exe --help
```

For a complete safe local command checklist, see
`docs/local-test-checklist.md`.

Quick local validation:

```powershell
.\scripts\test-local.ps1
```

The script runs `cargo fmt`, `cargo test`, `cargo build`, and fast read-only
agent commands. It does not start `serve`, does not start heartbeat loops, and
does not run heavy benchmarks.

## Commands

```sh
burd-agent system --json
burd-agent fit --json
burd-agent bench llm --provider ollama --model llama3.2:1b --runs 3 --json
burd-agent bench llm --provider vllm --url http://localhost:8000 --model Qwen/Qwen2.5-7B-Instruct --runs 3 --json
burd-agent bench stability --minutes 10 --json
burd-agent bench network --json
burd-agent bench disk --json
burd-agent score --json
burd-agent report --json
burd-agent report --run-all --json
burd-agent report --run-all --signed --json
burd-agent verify-report --file docs/examples/signed-report.json --json
burd-agent identity init
burd-agent identity show --json
burd-agent identity rotate-key --confirm
burd-agent api-token create --json
burd-agent api-token rotate --json
burd-agent api-token show --json
burd-agent challenge create-mock --json
burd-agent challenge run --file docs/examples/challenge.json --json
burd-agent challenge verify --file signed-response.json --json
burd-agent health --json
burd-agent heartbeat --once --json
burd-agent uptime --json
burd-agent uptime clear --confirm
burd-agent provider --json
burd-agent verify-provider --json
burd-agent readiness
burd-agent readiness --json
burd-agent pricing --json
burd-agent earnings --json
burd-agent actions --json
burd-agent logs --json
burd-agent logs --tail 50 --json
burd-agent history --json
burd-agent history latest --json
burd-agent history export --output history.json
burd-agent history clear --confirm
burd-agent registration-payload --json
burd-agent registration-payload --output registration.json
burd-agent raw --json
burd-agent serve --host 127.0.0.1 --port 8787
```

All commands with `--json` write valid JSON to stdout without mixed logs.

## Score

The Burd Compute Score is a 0-100 score with these MVP weights:

- 40% real LLM benchmark when available, or llmfit estimated throughput fallback;
- 20% VRAM and capacity;
- 15% stability;
- 10% network;
- 10% disk;
- 5% verification signals.

Tiers:

- `0-39`: Not Eligible
- `40-59`: Burd Basic
- `60-74`: Burd Plus
- `75-89`: Burd Pro
- `90-96`: Burd Max
- `97-100`: Burd Enterprise

Prices are demonstrative and marked with `prices_are_demonstrative: true` in
JSON reports.

## Provider Identity

Run:

```sh
burd-agent identity init
burd-agent identity show --json
```

The agent writes public config to `~/.burd/agent.json` and stores the Ed25519
private key separately at `~/.burd/agent.key`. Reports and raw data never expose
the private key.

For automation, set `BURD_AGENT_HOME` or `BURD_AGENT_CONFIG` to an alternate
workspace path.

## Signed Reports

Run:

```sh
burd-agent report --run-all --signed --json
```

The signed output includes:

- canonical report hash;
- Ed25519 signature;
- public key;
- key algorithm;
- signing timestamp;
- local signature verification result.
- canonicalization version.

`report --run-all` and `report --run-all --signed` append local benchmark
history to `~/.burd/benchmark-history.json`.

The default Rust test suite includes fast contract tests for signed reports,
local challenge responses, registration payloads, benchmark history, API token
status, raw/config redaction, and provider readiness. They use deterministic
internal fixtures for `SystemReport`, `FitReport`, `ScoreReport`,
`SignedReport`, and provider details. Persistent state is isolated with
temporary `BURD_AGENT_HOME`/`BURD_AGENT_CONFIG` values, so tests never use the
real `~/.burd` directory.

One slower integration test exercises real local hardware detection. It is
ignored by the default `cargo test` run and can be executed intentionally:

```powershell
cargo test -p burd-bench real_hardware_detection_integration_is_available -- --ignored
```

CI runs `cargo test --workspace` and enforces a 15-second budget for the
precompiled `burd-bench` contract suite. Run the same performance guard locally:

```powershell
.\scripts\check-contract-test-time.ps1
```

Sanitized JSON snapshots protect the provider, raw-data, and registration
payload contracts. See `docs/json-contract-snapshots.md` before intentionally
updating them.

The `.github/workflows/real-hardware-integration.yml` workflow runs only through
manual dispatch on a self-hosted runner labeled `burd-hardware`.
Runner isolation, registration, and removal are documented in
`docs/real-hardware-runner.md`.

## Local API Token

Run:

```sh
burd-agent api-token create --json
```

The token is printed once. The config stores only a token hash. Use
`api-token rotate --json` to rotate it and `api-token show --json` to check
status without printing the token.

When local API auth is enabled, protected endpoints expect:

```txt
Authorization: Bearer <token>
```

## Challenge Response

Run:

```sh
burd-agent challenge create-mock --json
burd-agent challenge run --file docs/examples/challenge.json --json
```

The MVP challenge flow is local and mock-backed. It prepares the future backend
flow where Burd issues a nonce-bound challenge, receives a signed response, and
validates report hash, signature, expiry, and provider identity.

Local verification now checks expiry, nonce, required tests, minimum agent and
benchmark versions, signed report hash, signed report signature, and challenge
response signature.

## Benchmark History

Run:

```sh
burd-agent history --json
burd-agent history latest --json
burd-agent history export --output history.json
```

History stores benchmark summaries only. It must not include private keys, API
tokens, or raw credentials.

## Provider Registration Payload

Run:

```sh
burd-agent registration-payload --json
burd-agent registration-payload --output registration.json
```

This builds the future backend registration payload locally. It includes public
identity, latest signed report hash, score, tier, capabilities, pricing, and
verification summary. It does not submit anything to Burd and does not include
secrets.

## Provider Readiness

Run:

```sh
burd-agent readiness
burd-agent readiness --json
```

Readiness consolidates identity, signed report, challenge evidence, provider
verification, benchmark history, API token status, and raw-data redaction into
a `0-100` score, checks, warnings, and recommendations.

`ready_locally` means all local checks pass. It does not mean backend
verification, audit approval, or marketplace acceptance. See
`docs/provider-readiness.md` for the complete contract and status definitions.

## Provider Console API and UI

Run:

```powershell
.\target\debug\burd-agent.exe serve --host 127.0.0.1 --port 8787
```

Endpoints:

- `GET /health`
- `GET /api/v1/system`
- `GET /api/v1/fit`
- `GET /api/v1/score`
- `GET /api/v1/report`
- `GET /api/v1/provider`
- `GET /api/v1/readiness`
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
- `POST /api/v1/benchmark/run`
- `POST /api/v1/challenge/run`
- `GET /api/v1/benchmark/status`

The local Provider Console UI is in `apps/benchmark-ui` and is served at `/` by
the local API. It follows the dark technical Burd design system from `SKILL.md`
and includes Overview, Hardware, Benchmarks, History, Uptime, Security,
Readiness, Registration, Logs, and Raw Data.

Open the UI in a browser at:

```txt
http://127.0.0.1:8787/
```

Stop the foreground server with `Ctrl+C`.

If a local process is stuck during development, stop it from PowerShell:

```powershell
Get-Process burd-agent -ErrorAction SilentlyContinue | Stop-Process -Force
```

To test the API without leaving the server running:

```powershell
.\scripts\test-api.ps1
```

The script starts `serve` on `127.0.0.1:8787`, calls the primary GET endpoints,
records output under `tmp/test-output/`, and stops the server by PID in a
`finally` block.

## Limitations

- Marketplace listing, payments, billing, login, job orchestration, and remote
  customer workloads are intentionally out of scope.
- Backend verification, audit, challenge issuing, reputation, job execution,
  leases, payouts and billing are future work.
- Earnings and prices are demonstrative and must not be treated as promised
  revenue.
- Real LLM benchmarking requires a local Ollama, vLLM, or MLX-compatible
  endpoint.
- Network benchmark defaults to a public endpoint unless `--endpoint` is
  supplied; no Burd backend is required for this MVP.
- `burd-agent serve --host 0.0.0.0` is possible but should be paired with
  `burd-agent api-token create --json`; without a token it emits a strong
  warning.
- Current `cargo build` and `cargo test` may emit inherited warnings from
  `third_party/llmfit/llmfit-core`. They do not block the local build/test flow
  and are left untouched to preserve the upstream llmfit integration.

## Documentation

- `docs/architecture.md`
- `docs/current-state.md`
- `docs/local-test-checklist.md`
- `docs/provider-console-parity.md`
- `docs/security.md`
- `docs/provider-identity.md`
- `docs/challenge-response.md`
- `docs/benchmark-history.md`
- `docs/local-api.md`
- `docs/provider-registration.md`
- `docs/provider-console-ui.md`
- `docs/llmfit-adaptation.md`
- `docs/benchmark-profiles.md`
- `docs/json-contract-snapshots.md`
- `docs/real-hardware-runner.md`
- `docs/examples/`
