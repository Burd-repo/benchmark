# Current State

This document records the technical audit for the Provider Agent MVP before and
after the June 2026 reliability pass.

## Repository Shape

- Rust workspace with `burd-agent`, `burd-api-local`, `burd-bench`,
  `burd-hardware`, `burd-llmfit`, and `burd-protocol`.
- Static Provider Console UI in `apps/benchmark-ui`.
- `third_party/llmfit` is preserved as the fit and LLM benchmark foundation.
- `AGENTS.md`, `SKILL.md`, `NOTICE.md`, `LICENSE`, README, docs, and examples
  are present.

## Existing Commands

- `system --json`
- `fit --json`
- `bench llm --provider <provider> --model <model> --runs <n> --json`
- `bench stability --minutes <n> --json`
- `bench network --endpoint <url> --json`
- `bench disk --json`
- `score --json`
- `report --json`
- `report --run-all --json`
- `report --run-all --signed --json`
- `verify-report --file <path> --json`
- `identity init`
- `identity show --json`
- `identity rotate-key --confirm`
- `challenge create-mock --json`
- `challenge run --file <path> --json`
- `challenge verify --file <path> --json`
- `health --json`
- `heartbeat --once --json`
- `provider --json`
- `verify-provider --json`
- `pricing --json`
- `earnings --json`
- `actions --json`
- `logs --json`
- `raw --json`
- `serve --host <ip> --port <port>`

## Added In This Stage

- `history --json`
- `history latest --json`
- `history clear --confirm`
- `history export --output <file>`
- `api-token create --json`
- `api-token rotate --json`
- `api-token show --json`
- `registration-payload --json`
- `registration-payload --output <file>`
- `uptime --json`
- `uptime clear --confirm`
- `logs --tail <n> --json`

## Functional Before This Stage

- Hardware detection through `burd-hardware` and `llmfit`.
- Model fit analysis through the `burd-llmfit` adapter.
- LLM benchmark execution through llmfit targets when runtimes are available.
- Stability, network, and disk benchmark modules.
- Burd Compute Score calculation.
- Local Ed25519 identity generation and key rotation.
- Signed reports with canonical JSON hash and Ed25519 signature.
- Local signed report verification.
- Mock challenge creation and local challenge response signing.
- Provider details, pricing, earnings estimate, verification summary.
- Local API and static Provider Console.
- Local actions/logs persisted to `~/.burd/actions.json` and `~/.burd/logs.json`.
- Local heartbeat persisted to `~/.burd/uptime.json`.

## Functional After This Stage

- Benchmark history persists to `~/.burd/benchmark-history.json`.
- `report --run-all` and `report --run-all --signed` append history entries.
- Challenge runs append signed benchmark history entries with `challenge_id`.
- Signed report envelope includes `canonicalization_version`.
- Challenge policy validates expiry, nonce, required tests, signed reports,
  report hash, signatures, and minimum versions locally.
- Local API token hash is stored in `~/.burd/agent.json`.
- Sensitive API endpoints check `Authorization: Bearer <token>` when auth is
  enabled.
- Registration payload is generated locally without secrets.
- Uptime can be listed and cleared through CLI.
- Network benchmark includes latency aliases, request counts, status code,
  DNS timing, duration, jitter, and warnings.
- Raw data includes explicit redaction metadata and summaries for history,
  signed reports, uptime, actions, logs, verification, pricing, and earnings.
- Provider Console includes Overview, Hardware, Benchmarks, History, Uptime,
  Security, Registration, Logs, and Raw Data.
- Local API contract tests cover health, lightweight public endpoints,
  protected endpoint 401 behavior when token auth is enabled, and config/raw
  redaction expectations without starting `serve`.

## Still Mocked Or Future

- Backend-issued challenges and backend verification.
- Burd audit service and production antifraud.
- Marketplace listings, leases, jobs, orchestration, scheduler, and containers.
- Real earnings, payouts, billing, Pix, and financial settlement.
- Remote database or production cloud backend.
- Reputation and provider marketplace ranking.
- Hardware attestation through TPM/HSM/OS keychain.

## Known Build Warnings

- `cargo build` and `cargo test` can emit inherited warnings from
  `third_party/llmfit/llmfit-core`, currently around unused llmfit internals.
- These warnings do not block the Burd local build/test flow.
- They are intentionally left in `third_party/llmfit` unless a safe upstream
  compatibility-preserving fix is needed.

## Persisted Local Data

- `~/.burd/agent.json`: public identity and local config, API token hash only.
- `~/.burd/agent.key`: Ed25519 private key, never exposed in reports/raw/history.
- `~/.burd/latest-report.json`: latest unsigned/full report.
- `~/.burd/latest-signed-report.json`: latest signed report envelope.
- `~/.burd/benchmark-history.json`: benchmark history summaries.
- `~/.burd/uptime.json`: heartbeat history.
- `~/.burd/actions.json`: action records.
- `~/.burd/logs.json`: task logs.
