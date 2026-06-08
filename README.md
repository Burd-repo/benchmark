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
burd-agent challenge create-mock --json
burd-agent challenge run --file docs/examples/challenge.json --json
burd-agent challenge verify --file signed-response.json --json
burd-agent health --json
burd-agent heartbeat --once --json
burd-agent provider --json
burd-agent verify-provider --json
burd-agent pricing --json
burd-agent earnings --json
burd-agent actions --json
burd-agent logs --json
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

## Challenge Response

Run:

```sh
burd-agent challenge create-mock --json
burd-agent challenge run --file docs/examples/challenge.json --json
```

The MVP challenge flow is local and mock-backed. It prepares the future backend
flow where Burd issues a nonce-bound challenge, receives a signed response, and
validates report hash, signature, expiry, and provider identity.

## Provider Console API and UI

Run:

```sh
burd-agent serve --host 127.0.0.1 --port 8787
```

Endpoints:

- `GET /health`
- `GET /api/v1/system`
- `GET /api/v1/fit`
- `GET /api/v1/score`
- `GET /api/v1/report`
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

The local Provider Console UI is in `apps/benchmark-ui` and is served at `/` by
the local API. It follows the dark technical Burd design system from `SKILL.md`
and includes Overview, Hardware, Benchmarks, Jobs/Leases future, Earnings,
Uptime, Security, Logs, and Raw Data.

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
- `burd-agent serve --host 0.0.0.0` is possible but emits a warning because API
  token support is future work.

## Documentation

- `docs/architecture.md`
- `docs/provider-console-parity.md`
- `docs/security.md`
- `docs/provider-identity.md`
- `docs/challenge-response.md`
- `docs/local-api.md`
- `docs/provider-console-ui.md`
- `docs/llmfit-adaptation.md`
- `docs/benchmark-profiles.md`
- `docs/examples/`
