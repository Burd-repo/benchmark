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
- Production cloud deployment and complete remote backend operations.
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
- Protected endpoints without a token are surfaced as Token required; the UI does not implement token entry yet.
- Benchmark execution remains a manual heavy action with confirmation and is not run automatically.

## PR 12 - Documentation Consolidation

- Provider Trust Layer documentation now explicitly defines readiness, compute, network, reliability, trust, verification status, capability spot, and workload eligibility as separate local concepts.
- Dedicated docs cover reliability score, network score, trust score, local/mock spot verification, and workload eligibility contracts.
- README and current docs clarify the NVIDIA/CUDA-only marketplace MVP policy, AMD/ROCm/Vulkan local-diagnostic behavior, evidence expiration, session invalidation, heartbeat boundaries, and future scheduler/marketplace consumption.
- The documentation keeps backend registry, remote Proof of Capability, marketplace listings, jobs, leases, orchestration, Pix, billing, and payouts out of the local MVP.

## BN-00 - Architecture Freeze

- BN-00 freezes the first Burd Network backend boundary without implementing
  backend runtime code.
- `docs/bn-00-architecture-freeze.md` records scope, non-goals, backend
  direction, and the BN-01 gate.
- `docs/adr/0001-control-plane-modular-monolith.md` accepts the Rust modular
  monolith control plane with PostgreSQL, object storage, simple queue, and
  outbound provider connections.
- `docs/remote-protocol-v1.md` defines the initial `/v1` enrollment, session,
  evidence, challenge, telemetry, error, idempotency, and audit contracts.
- `docs/remote-authority-matrix.md` defines which fields are agent-claimed,
  agent-signed evidence, backend-attested, backend-derived, or never accepted.
- `docs/threat-model.md` defines assets, actors, trust boundaries, threats,
  controls, privacy boundaries, and residual risk for BN-01 through BN-09.

## BN-01 - Backend Foundation

- `crates/burd-control-plane` adds the first Rust backend foundation crate.
- The control plane exposes `GET /health`, `GET /ready`, `GET /openapi.json`,
  `POST /v1/providers`, and `GET /v1/providers/{provider_id}`.
- Configuration is environment-driven through `BURD_CONTROL_*` variables.
- PostgreSQL migrations create the initial registry, evidence, session,
  idempotency, and audit tables.
- Provider creation persists a real provider row, writes an audit event, and
  stores an idempotency result.
- Mutating provider creation requires `Idempotency-Key` and uses the BN-00 error
  envelope for failures.
- A lightweight in-memory rate limiter protects the HTTP surface.
- Unit tests cover config parsing, error codes, migrations, OpenAPI, rate
  limiting, request hashing, and health routing. The PostgreSQL persistence test
  is isolated by schema and ignored unless a test database URL is provided.
- BN-01 does not implement remote enrollment, device credentials, control
  channel, backend-issued challenges, telemetry ingestion, trust policy,
  scheduler, jobs, marketplace, billing, Pix, or payouts.

## BN-02 - Remote Provider Enrollment And Identity

- The control plane issues short-lived one-time enrollment tokens behind an
  admin bearer credential.
- The agent submits its public key, machine ID, registration payload, versions,
  and hardware fingerprint, then signs a backend-issued nonce.
- Canonical `burd.enrollment-proof.v1` claims bind provider, enrollment,
  machine, key, fingerprint, nonce, and server expiry.
- Successful proof creates backend-attested provider/device IDs, an active
  Ed25519 key, audit history, and a short-lived device credential.
- PostgreSQL stores only token and credential hashes, never raw values.
- Credential refresh, device listing, key rotation, and cascading device
  revocation are implemented.
- `burd-agent enrollment enroll`, `status`, and `refresh-credential` implement
  the agent side without exposing credentials in status or action logs.
- PostgreSQL integration tests cover enrollment, replay rejection, credential
  rotation, key rotation, and revocation in isolated schemas.
- BN-02 does not implement the remote session/control channel, heartbeat,
  telemetry, Proof of Capability, trust policy, jobs, scheduler, marketplace,
  billing, Pix, or payouts.
## BN-03 - Remote Session And Control Channel

- The control plane creates or resumes one nonterminal remote session per
  enrolled device and stores only a hash of the resume token.
- The agent opens an authenticated outbound WebSocket, so providers do not need
  to expose an inbound public port.
- Heartbeats are monotonic, server-timestamped, persisted, and bound to the
  enrollment hardware fingerprint.
- The backend derives `online`, `offline`, `degraded`, `expired`, and `revoked`;
  a periodic server task expires stale sessions independently of requests.
- Duplicate sockets and replayed sequences are rejected. Sequence gaps or
  fingerprint mismatch degrade the session.
- `burd-agent remote-session connect` maintains the connection with credential
  refresh and reconnect backoff. `remote-session status` reads backend state.
- PostgreSQL integration coverage exercises start, duplicate rejection,
  heartbeat, degradation, resume, and revocation.
- BN-03 does not implement GPU telemetry, backend-issued challenges, Proof of
  Capability, trust policy, jobs, scheduler, marketplace, billing, Pix, or
  payouts.
## BN-04 - Signed GPU Telemetry

- The agent collects NVIDIA GPU identity and live metrics through grouped,
  structured `nvidia-smi` CSV queries that tolerate unsupported optional fields.
- GPU UUID, PCI identity, compute capability, driver/CUDA compatibility, VRAM,
  utilization, temperature, power, clocks, throttle reasons, ECC, and redacted
  compute-process data are represented in the versioned telemetry contract.
- Telemetry batches bind provider, device, session, fingerprint, control
  sequence, sample sequence range, canonical hash, active key ID, and Ed25519
  signature.
- The control plane verifies signatures and physical ranges, enforces batch
  size/frequency/clock policy, persists normalized samples transactionally, and
  returns `telemetry_ack` over WebSocket or HTTP fallback.
- Process paths are reduced to basenames; command lines are not collected.
- Server-time retention removes old batches and cascades their samples.
- `burd-agent remote-session connect --telemetry` enables collection without
  making telemetry a replacement for heartbeat liveness.
- BN-04 does not implement DCGM, challenge-bound telemetry windows, regional
  probes, global trust/antifraud, jobs, scheduler, marketplace, or billing.
