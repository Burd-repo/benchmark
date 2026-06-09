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
- `readiness`
- `readiness --json`
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
  Security, Readiness, Registration, Logs, and Raw Data.
- Provider Readiness consolidates identity, signed report, challenge evidence,
  verification, history, API token status, and raw redaction into a local
  score, checks, warnings, and recommendations.
- VRAM detection records optional source and confidence metadata and prioritizes
  real driver/system/device measurements over llmfit known-GPU estimates.
- Vulkan fallback reads device-local memory heaps for discrete GPUs when
  available. Name-table VRAM remains a final `estimated` fallback only.
- System reports, signed reports, provider details, raw data, registration
  payloads, and provider verification preserve VRAM source/confidence metadata.
- Local API contract tests cover health, lightweight public endpoints,
  protected endpoint 401 behavior when token auth is enabled, and config/raw
  redaction expectations without starting `serve`.
- Persistent local contract tests cover signed reports, local challenge
  verification, registration payloads, benchmark history, API token status,
  raw/config redaction, and provider readiness states.
- Contract tests create isolated temporary `BURD_AGENT_HOME` and
  `BURD_AGENT_CONFIG` paths. They do not read or write the user's real
  `~/.burd` directory and clean up the temp state automatically.

## Still Mocked Or Future

- Backend-issued challenges and backend verification.
- Burd audit service and production antifraud.
- Marketplace listings, leases, jobs, orchestration, scheduler, and containers.
- Real earnings, payouts, billing, Pix, and financial settlement.
- Remote database or production cloud backend.
- Reputation and provider marketplace ranking.
- Hardware attestation through TPM/HSM/OS keychain.
- Production marketplace policy that requires detected/high-confidence VRAM and
  rejects or limits estimated capacity.

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

## Contract Test Coverage

The current local test suite protects the Provider Agent contracts without
starting the API server or depending on host state:

- signed report envelope fields, canonical hash, Ed25519 signature, local
  verification, and absence of private key/API token material;
- mock challenge response shape, nonce binding, expiry handling, required test
  failures, wrong nonce rejection, signed report hash/signature checks, and
  absence of secret material;
- registration payload shape, latest signed report hash, public identity,
  capabilities, pricing, verification summary, and `secrets_included: false`;
- benchmark history empty/list/latest/export behavior, signed report metadata,
  challenge IDs, warnings, and no credential material;
- API token status, token verification success/failure, protected endpoint
  valid/invalid token behavior, and redacted config/raw payloads;
- local provider readiness classifications for `uninitialized`,
  `not_verified`, `ready_locally`, and `failed` states.
