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
- `fingerprint --json`
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
- `identity migrate --confirm`
- `identity migrate --from <state-directory> --confirm`
- `identity show --json`
- `identity rotate-key --confirm`
- `challenge create-mock --json`
- `challenge run-local --json`
- `challenge run --file <path> --json`
- `challenge verify --file <path> --json`
- `session start --json`
- `session status --json`
- `session stop --json`
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
- `trust-score --json`
- `capability-spot --json`
- `workload-eligibility --json`
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
- Challenge runs persist a complete response bundle; readiness revalidates it
  and does not trust a history `challenge_id` alone.
- State resolution is canonical: `BURD_AGENT_CONFIG` and its parent directory
  take precedence, then `BURD_AGENT_HOME`, then `~/.burd`.
- Identity migration backs up, validates, normalizes, repairs, or imports local
  state without silently overwriting the previous target state.
- Signed report envelope includes `canonicalization_version`.
- Challenge policy validates expiry, nonce, required tests, signed reports,
  report hash, signatures, and minimum versions locally.
- Local API token hash is stored in `~/.burd/agent.json`.
- Sensitive API endpoints check `Authorization: Bearer <token>` when auth is
  enabled.
- Registration payload is generated locally without secrets.
- Uptime can be listed and cleared through CLI.
- Uptime summaries include `uptime_score` and `uptime_level`.
- Local reliability can be calculated from heartbeat history through CLI and API.
- Provider details, raw data, registration payloads, full reports, signed reports, and the Provider Console surface local reliability.
- Local network score can be calculated from the latest finite network benchmark through CLI and API.
- Provider details, raw data, registration payloads, full reports, signed reports, and the Provider Console surface local network score.
- Local trust score can be calculated from verification, evidence freshness, reliability, network quality, and local benchmark history through CLI and API.
- Provider details, raw data, and registration payloads surface local trust score summaries.
- Local/mock AI capability spot verification can be calculated from fit analysis, runtime readiness, signed evidence, optional local LLM benchmark proof, and local history depth through CLI and API.
- Provider details, raw data, and registration payloads surface local/mock capability spot verification summaries.
- Local AI performance metrics consolidate measured LLM benchmark evidence, signed reports, benchmark history, and fit estimates through CLI and API without running benchmarks automatically.
- Local workload eligibility can be calculated from fit recommendations, capability spot verification, trust score, provider verification, reliability, compute score, and marketplace GPU policy through CLI and API.
- Provider details, raw data, and registration payloads surface local and future-marketplace workload eligibility summaries.
- Network benchmark includes latency aliases, request counts, status code,
  DNS timing, duration, jitter, and warnings.
- Raw data includes explicit redaction metadata and summaries for history,
  signed reports, uptime, actions, logs, verification, pricing, and earnings.
- Provider Console includes Overview, Hardware, Benchmarks, History, Uptime,
  Security, Readiness, Registration, Logs, and Raw Data.
- Provider Readiness consolidates identity, signed report, challenge evidence,
  verification, history, API token status, and raw redaction into a local
  score, checks, warnings, and recommendations.
- A versioned SHA-256 hardware fingerprint is generated from stable hardware,
  backend, VRAM evidence, and driver signals. It is propagated through signed
  reports, provider details, challenge responses, and registration payloads.
- Provider verification and readiness surface a mismatch between current
  hardware and the latest signed report or challenge response.
- The local `nvidia_cuda_only_mvp` policy marks supported NVIDIA RTX 30xx+ and
  compatible datacenter GPUs as potentially marketplace eligible only when
  CUDA and detected/reliable VRAM evidence are present.
- AMD, Intel, Apple Silicon, ROCm, Vulkan-only, CPU-only, unsupported NVIDIA,
  and unreliable-VRAM systems remain diagnostic-only or not eligible for the
  future paid marketplace.
- Provider Session persists a local expirational session snapshot with
  `active`, `expired`, `invalidated`, `stopped`, `failed`, and `inactive`
  states. It records the readiness snapshot, report hash, challenge id,
  hardware fingerprint, and marketplace policy snapshot at start time.
- Heartbeat once records a one-shot local liveness snapshot for an active
  session, appends uptime history, and propagates a heartbeat summary into
  provider details, raw data, registration payloads, and readiness.
- Full and signed reports have a 7-day local TTL. Challenges and challenge
  responses have a 24-hour local TTL.
- Freshness contracts expose issuance, expiry, age, TTL, and current expiry
  state. Verifiers recalculate dynamic freshness instead of trusting persisted
  flags.
- Readiness distinguishes missing, invalid, expired, and valid report/challenge
  evidence. Expired evidence receives no readiness points.
- Provider verification and registration payloads expose current evidence
  freshness. Expired signed benchmarks do not count as verified benchmarks.
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
- Production marketplace policy that evolves beyond the initial local
  `nvidia_cuda_only_mvp` classification.
- Backend-bound availability and scheduler enforcement of workload eligibility.
- Remote Proof of Capability and backend-attested AI performance verification.

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
- `~/.burd/latest-challenge-response.json`: latest complete local challenge
  response and verification bundle.
- `~/.burd/provider-session.json`: local provider session snapshot.
- `~/.burd/benchmark-history.json`: benchmark history summaries.
- `~/.burd/uptime.json`: heartbeat history.
- `~/.burd/latest-network.json`: latest finite network benchmark sample.
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
- one-shot heartbeat behavior, local uptime history updates, active-session
  liveness checks, and session invalidation on fingerprint mismatch.

## Provider Console Integration - PR 11

- Provider Console keeps the existing visual structure and adds direct consumption of AI Performance, Local Trust Score, Capability Spot, and Workload Eligibility endpoints.
- AI Performance is integrated into Benchmarks; Workloads is a new tab for local and future-marketplace workload eligibility.
- Trust and Capability are shown as local heuristic/local mock signals, not remote marketplace verification or Proof of Capability.
- Protected endpoints without a token are surfaced as `Token required`; the UI does not implement token entry yet.
- Benchmark execution remains a manual heavy action with confirmation and is not run automatically.