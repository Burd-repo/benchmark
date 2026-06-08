# Burd Benchmark

Burd Benchmark is the local validation product for the Burd GPU marketplace. It
installs as `burd-agent`, detects provider hardware, estimates which AI workloads
fit, runs real local inference benchmarks when runtimes are available, calculates
a Burd Compute Score, and generates a structured report for future marketplace
validation.

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
burd-agent identity init
burd-agent serve --host 0.0.0.0 --port 8787
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

## Local API and UI

Run:

```sh
burd-agent serve --host 0.0.0.0 --port 8787
```

Endpoints:

- `GET /health`
- `GET /api/v1/system`
- `GET /api/v1/score`
- `GET /api/v1/report`
- `POST /api/v1/benchmark/run`
- `GET /api/v1/benchmark/status`

The local benchmark UI is in `apps/benchmark-ui` and is served at `/` by the
local API. It follows the dark technical Burd design system from `SKILL.md`.

## Local Identity

`burd-agent identity init` creates the local config at `~/.burd/agent.json` and
writes a private-key placeholder next to it. For automation and tests, set
`BURD_AGENT_CONFIG` to an alternate file path.

## Limitations

- Marketplace listing, payments, billing, login, job orchestration, and remote
  customer workloads are intentionally out of scope.
- Antifraud signatures and backend challenges are MVP placeholders.
- Real LLM benchmarking requires a local Ollama, vLLM, or MLX-compatible
  endpoint.
- Network benchmark defaults to a public endpoint unless `--endpoint` is
  supplied; no Burd backend is required for this MVP.

## Documentation

- `docs/architecture.md`
- `docs/llmfit-adaptation.md`
- `docs/benchmark-profiles.md`
- `docs/examples/`
