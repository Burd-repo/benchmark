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
- `runtime check --json`
- `runtime plan --image-ref <digest-pinned-image> --allow-image-ref <digest-pinned-image> --gpu-uuid <gpu-uuid> --json`
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
- Secure runtime planning can inspect Docker/NVIDIA readiness and produce a hardened Docker sandbox plan for digest-pinned, allowlisted runtime images without executing customer jobs.
- The control plane can create backend-authorized compute jobs for a specific provider/device/session, let an authenticated provider session pull the next job, issue job-scoped data-plane grants, record progress events, accept final result metadata, and cancel non-terminal jobs.
- The control plane can create customer users, organizations, projects, quotas, hashed customer API keys, credit ledger entries, marketplace reservations, usage summaries, and customer audit events. Reservations are scoped to backend-published marketplace listings and project quotas.
- The control plane exposes operational observability with correlation IDs, structured JSON logs, Prometheus metrics, admin snapshots, background task error counters, and configurable HTTP SLO status.
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

- Agent-side proof workload execution, backend benchmark profile runners/submission automation, background verification scheduler automation, and production regional probe workers.
- Production antifraud operations, case review, admin resolution, and automated enforcement.
- Marketplace checkout/orchestration beyond single-listing reservation, autonomous/background scheduling, paid job container execution, byte-level data-plane transfer, billing-grade metering enforcement, and external financial settlement.
- Real Pix gateway capture, bank payout execution, earnings settlement, refunds, disputes, tax workflows, and production financial reconciliation.
- Production cloud deployment, external observability export, dashboards-as-code, alerting, automated backup/restore, and complete remote backend operations.
- Reputation and provider marketplace ranking.
- Hardware attestation through TPM/HSM/OS keychain.
- Production marketplace policy that evolves beyond the initial local
  `nvidia_cuda_only_mvp` classification.
- Production scheduler optimization, marketplace demand matching, reservations across supply inventory, and multi-GPU/multi-provider placement. BN-14 only offers leases for already-created, already-targeted jobs.
- Agent-side Proof of Capability execution, agent-side Benchmark Profiles v2 runners, deployed probe workers, provider-side job execution, and production risk model inputs beyond BN-19 state.

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
  controls, privacy boundaries, and residual risk for BN-01 through BN-19.

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

## BN-05 - Remote Evidence Registry

- The control plane accepts signed evidence through
  `POST /v1/sessions/{session_id}/evidence-records` using the same device
  credential, session token, and device ID headers as remote session APIs.
- `SignedReport` envelopes are verified server-side for canonical report hash,
  envelope hash, Ed25519 signature, active backend key binding, enrolled
  machine ID, backend/local provider binding, and session hardware fingerprint.
- The backend recalculates evidence freshness from `signed_at` and server time;
  it does not trust `is_expired` or `signature_valid_locally` sent by the agent.
- Complete signed envelopes are stored in filesystem-backed object storage for
  local/dev deployments, controlled by `BURD_CONTROL_OBJECT_STORAGE_DIR`.
- PostgreSQL `evidence_records` metadata now includes session, public key,
  report hash, fingerprint, subject, revocation, and verification fields.
- Evidence hashes are globally deduplicated; replaying the same envelope returns
  the existing record with `duplicate=true`.
- Admin endpoints list, read, and revoke evidence records. Revocation updates
  metadata and audit history without deleting the stored envelope.
- Accepted signed reports also create hardware snapshot rows for later policy,
  trust, and antifraud consumers.
- BN-05 does not implement active Proof of Capability, recurring verification,
  trust/antifraud scoring, jobs, scheduler, marketplace, billing, Pix, or payouts.

## BN-06 - Active Proof Of Capability Protocol

- `burd-protocol` defines backend-issued `ProofCapabilityChallenge` and signed
  `SignedProofCapabilityResponse` contracts with canonical response hashing and
  the `burd.proof-capability-response.v1` signature domain.
- The control plane exposes `POST /v1/challenges` for admin challenge issuance,
  `GET /v1/challenges/{challenge_id}` for backend state inspection,
  `GET /v1/sessions/{session_id}/challenges/next` for session-authenticated
  pickup, and `POST /v1/sessions/{session_id}/challenges/{challenge_id}/response`
  for signed response submission.
- Challenge issuance requires an `online` or `degraded` remote session and a
  `required_fingerprint` matching the backend session fingerprint.
- PostgreSQL `proof_challenges` stores status, nonce, required GPU/backend/
  artifact/prompt fields, thresholds, response hash, public key ID, object key,
  response envelope, and verification JSON.
- Full signed proof responses are written to filesystem-backed object storage
  under `proof-challenges/{provider_id}/{challenge_id}/{response_hash}.json`.
- Backend verification recalculates response hash, verifies Ed25519 against the
  active backend device key, checks server-side expiry, binds provider/device/
  session/fingerprint/GPU/backend/artifact/prompt, and evaluates initial CUDA,
  VRAM, GEMM, LLM metric, contention, and telemetry-window proof fields.
- Audit events cover challenge issuance, acknowledgement, verification failure,
  verification success, and expiration-by-server-clock.
- BN-06 does not implement the agent-side CUDA/VRAM/GEMM/LLM workload runner,
  recurring verification policy state, global trust/antifraud scoring, jobs,
  scheduler, marketplace, billing, Pix, or payouts. BN-07 adds the recurring
  verification state and admin sweep around BN-06.

## BN-07 - Recurring And Risk-Based Verification

- `burd-protocol` defines verification sweep and verification-state response contracts plus `burd-verification-policy-v1`.
- PostgreSQL migration `0007_recurring_verification_policy` adds `provider_verification_states` and policy metadata on `proof_challenges`.
- The control plane exposes `POST /v1/verification/sweep` for admin-triggered recurring/risk verification and `GET /v1/providers/{provider_id}/verification-states` for state inspection.
- The sweep evaluates online/degraded sessions, skips blocked/quarantined providers and inactive devices, avoids duplicate active challenges, and issues BN-06 challenges when devices are new, due, suspect, forced, or stale-running.
- Challenge expiry is recalculated by server time during sweeps. Expired running verifications become failed verification state.
- BN-06 proof responses now update provider-device verification state to `verified`, `verification_due`, or `suspect` in the same transaction as challenge verification.
- Config controls period, retry budget, sweep limit, and suspect failure threshold through `BURD_CONTROL_VERIFICATION_*` variables.
- BN-07 does not implement the agent-side proof workload runner, autonomous background scheduler process, global trust/antifraud model, jobs, scheduler, marketplace, billing, Pix, or payouts.

## BN-08 - Regional Network Probes

- `burd-protocol` defines trusted regional network probe observation, observation history, regional reachability, and provider network state contracts.
- PostgreSQL migration `0008_regional_network_probes` adds `network_probe_observations` and `provider_network_states`.
- The control plane exposes `POST /v1/network-probes/observations`, `GET /v1/providers/{provider_id}/network-probes`, and `GET /v1/providers/{provider_id}/network-state` behind admin/probe authorization.
- Probe observations are tied to existing provider, device, and remote session IDs. The backend rejects blocked/quarantined providers, inactive devices, unsupported session states, invalid metrics, future timestamps, and non-redacted metadata.
- Duplicate observations are deduplicated by `(session_id, probe_id, observed_at)` and do not inflate score history.
- The backend calculates `remote_network_score`, `regional_reachability`, and `effective_network_score`; providers do not decide their own remote network reputation.
- BN-08 does not deploy production multi-region probe workers, add separate probe credentials, run real data-plane artifact probes, feed scheduler decisions, or implement jobs, marketplace, billing, Pix, or payouts.

## BN-09 - Global Trust And Antifraud

- `burd-protocol` defines backend trust sweep, provider trust state, and antifraud event contracts.
- PostgreSQL migration `0009_global_trust_antifraud` adds `provider_trust_states` and `antifraud_events`.
- The control plane exposes `POST /v1/trust/sweep`, `GET /v1/providers/{provider_id}/trust-states`, and `GET /v1/providers/{provider_id}/antifraud-events` behind admin authorization.
- Trust state is recalculated from backend-owned provider/device state, latest remote session, heartbeat history, telemetry presence, evidence records, proof challenge history, recurring verification state, and regional network state.
- The backend stores `trust_score`, `risk_score`, backend reliability score, trust status, reason codes, and active antifraud events; providers do not decide their own global trust or risk.
- Cold start remains separate through `new_provider` and `insufficient_history`, so missing history is not treated as fraud by itself.
- Initial antifraud signals cover heartbeat without telemetry, degraded sessions, repeated proof failures, weak remote network, duplicate GPU UUIDs, hardware fingerprint reuse, suspect verification, and missing evidence/proof.
- BN-09 does not implement production antifraud case management, automatic quarantine/block enforcement, scheduler consumption, marketplace ranking, jobs, leases, billing, Pix, or payouts.

## BN-10 - Benchmark Profiles v2

- `burd-protocol` defines versioned benchmark profile records, signed benchmark result envelopes, canonical result hashing, signature messages, verification records, and list/submit responses.
- PostgreSQL migration `0010_benchmark_profiles_v2` adds `benchmark_profiles` and `benchmark_results`.
- The control plane exposes `POST /v1/benchmark-profiles`, `GET /v1/benchmark-profiles`, `POST /v1/sessions/{session_id}/benchmark-results`, and `GET /v1/providers/{provider_id}/benchmark-results`.
- Benchmark profiles include workload type, image digest, optional model/artifact hashes, backend, minimum VRAM, redacted parameters, warmup/duration/sample count, thresholds, and lifecycle status.
- Signed benchmark results bind provider, device, session, run ID, profile ID/version, backend, hardware fingerprint, GPU UUID, image digest, optional model/artifact hashes, profile configuration, metrics, telemetry window hash, result hash, active key ID, canonicalization version, and Ed25519 signature.
- The backend verifies result hash, active device-key signature, remote-session binding, hardware fingerprint, active profile binding, backend binding, image/model/artifact binding, profile timing/parameter binding, timestamps, metric ranges, and threshold satisfaction.
- Results below profile thresholds are stored as `failed`; valid results meeting thresholds are stored as `succeeded`.
- BN-10 does not implement agent-side benchmark profile runners, versioned container execution, scheduler consumption, jobs, leases, marketplace, billing, Pix, or payouts. BN-11 adds backend workload eligibility v2.

## BN-11 - Remote Policy And Workload Eligibility v2

- `burd-protocol` defines backend-owned workload policy requirements, workload policy records, eligibility records, sweep requests, and list responses.
- PostgreSQL migration `0011_workload_eligibility_v2` adds `workload_policies` and `provider_workload_eligibility`.
- The control plane exposes `POST /v1/workload-policies`, `GET /v1/workload-policies`, `POST /v1/workload-eligibility/sweep`, and `GET /v1/providers/{provider_id}/workload-eligibility` behind admin authorization.
- Eligibility is recalculated from provider/device state, latest remote session, verification state, global trust/risk/reliability state, regional network state, signed GPU telemetry, signed benchmark results, and backend policy requirements.
- Stored statuses are `eligible`, `limited`, `ineligible`, `verification_required`, `temporarily_unavailable`, and `blocked`, with persisted reason codes and audit events.
- The provider cannot submit or self-approve remote eligibility. Local workload eligibility remains diagnostic; BN-11 eligibility is backend-derived state for future scheduler and marketplace use.
- BN-11 does not implement scheduler enforcement, secure provider runtime, jobs, leases, marketplace listings, billing, Pix, payouts, or autonomous production sweep scheduling.
## BN-12 - Secure Provider Runtime

- `burd-protocol` defines `SecureRuntimePlan`, runtime checks, image allowlist entries, resource limits, tmpfs mounts, and a hardened security profile.
- `burd-bench` builds local secure runtime plans from host probes, Docker/NVIDIA runtime availability, GPU UUID binding, template allowlist, digest-pinned image references, resource limits, and security defaults.
- `burd-agent runtime check --json` returns a diagnostic plan for the current host without requiring an image.
- `burd-agent runtime plan --image-ref <image@sha256:digest> --allow-image-ref <image@sha256:digest> --gpu-uuid <gpu_uuid> --json` emits Docker arguments only when the plan status is `ready`.
- Ready plans require Linux, Docker, NVIDIA Container Toolkit runtime advertising, an approved template, a digest-pinned allowlisted image, a GPU UUID, valid limits, read-only rootfs, non-root user, dropped capabilities, no-new-privileges, seccomp, no network, no IPC sharing, explicit tmpfs mounts, ephemeral secrets mode, and cleanup requirement.
- BN-12 does not implement job submission, backend leases, customer artifact download, result upload, arbitrary shell execution, metering, scheduler, marketplace, billing, Pix, payouts, Kubernetes, or distributed workloads.

## BN-13 - Job API And Data Plane

- `burd-protocol` defines job artifacts, job records, create/list/next/accept/event/result/cancel contracts, and job-scoped data-plane grants.
- PostgreSQL migration `0012_job_api_data_plane` adds `compute_jobs` and `job_events`.
- The control plane exposes `POST /v1/jobs`, `GET /v1/jobs/{job_id}`, `GET /v1/providers/{provider_id}/jobs`, `POST /v1/jobs/{job_id}/cancel`, `GET /v1/sessions/{session_id}/jobs/next`, `POST /v1/sessions/{session_id}/jobs/{job_id}/accept`, `POST /v1/sessions/{session_id}/jobs/{job_id}/events`, and `POST /v1/sessions/{session_id}/jobs/{job_id}/result`.
- Job creation is admin-authorized, idempotent, scoped to one provider/device/session/GPU, and requires an online or degraded session plus backend workload eligibility of `eligible` or `limited`.
- Jobs are limited to approved templates, CUDA backend, digest-pinned images, structured artifact manifests, and redacted JSON parameters/metadata.
- Provider sessions pull a job over authenticated outbound session credentials and receive a job-scoped data-plane grant with separate credential material and scoped artifact paths. BN-14 now requires an offered scheduler lease before this pull can assign work.
- BN-13 does not implement scheduler selection, leases, provider-side container execution, byte-level artifact transfer, object-storage signed URLs, metering, billing, Pix, payouts, marketplace listing, multi-GPU jobs, or multi-provider jobs.

## BN-14 - Scheduler And Leases

- `burd-protocol` defines job lease records, scheduler run requests/responses, scheduler decisions, and lease list responses.
- PostgreSQL migration `0013_scheduler_leases` adds `job_leases` plus active-lease uniqueness for one job and one provider/device/GPU.
- The control plane exposes `POST /v1/scheduler/run`, `GET /v1/jobs/{job_id}/leases`, and `GET /v1/providers/{provider_id}/leases` behind admin authorization.
- Scheduler runs are bounded and admin-triggered. They expire stale offered leases, scan queued jobs, require active device plus online/degraded session, require backend workload eligibility of `eligible` or `limited`, and offer short-lived leases.
- Provider `GET /v1/sessions/{session_id}/jobs/next` now consumes the oldest non-expired offered lease for that authenticated session, marks the job assigned, and returns the lease with the job-scoped data-plane grant.
- Lease status follows `offered -> accepted -> provisioning -> active -> completed | failed | expired` and is updated alongside job accept/event/result/cancel transitions.
- BN-14 does not implement autonomous scheduler daemon cadence, marketplace demand matching, paid provider-side execution, byte-level artifact transfer, metering, billing, Pix, payouts, multi-GPU placement, or multi-provider jobs.

## BN-15 - Metering And Usage Ledger

- `burd-protocol` defines job usage receipts, usage ledger entries, finalize responses, and list responses.
- PostgreSQL migration `0014_usage_metering_ledger` adds append-only `usage_ledger_entries` with a trigger that rejects updates and deletes.
- The control plane exposes `POST /v1/jobs/{job_id}/usage-ledger/finalize`, `GET /v1/jobs/{job_id}/usage-ledger`, and `GET /v1/providers/{provider_id}/usage-ledger` behind admin authorization.
- Terminal job result and cancel flows append a `job_usage_finalized` entry in the same transaction as job/lease terminal state.
- Usage receipts record reserved GPU seconds, actual GPU seconds, billable/non-billable metering basis, idle unbillable seconds, input/output/network/storage bytes, retry count, failure classification, challenge non-billable seconds, reason codes, receipt hash, and source hash.
- Replaying finalize for an already-metered job returns the existing ledger entry with `duplicate=true`; it does not mutate history.
- BN-15 does not implement customer balances, provider payables, invoices, Pix, payouts, double-entry financial accounting, byte-level artifact verification, signed receipt key management, disputes, refunds, or marketplace pricing.

## BN-16 - Marketplace Registry And Listings

- `burd-protocol` defines marketplace listing records, marketplace sweep responses, and list responses.
- PostgreSQL migration `0015_marketplace_registry_listings` adds materialized `marketplace_listings` with status, current status, verified GPU/VRAM flags, region, trust, reliability, verification, benchmark, pricing placeholder, availability, source hash, and indexes for marketplace search.
- The control plane exposes `POST /v1/marketplace/listings/sweep`, `GET /v1/marketplace/listings`, and `GET /v1/providers/{provider_id}/marketplace-listings` behind admin authorization.
- Listings are derived from backend workload eligibility, trust, verification, network, benchmark, session, device/provider, and active lease state. Providers cannot self-publish or self-mark verified marketplace inventory.
- `GET /v1/marketplace/listings` returns `published` and `limited` listings by default; provider-scoped reads expose all provider listing statuses for inspection.
- BN-16 does not implement customer accounts, reservations, marketplace checkout, provider-set pricing, billing, Pix, payouts, SLA contracts, or financial settlement.

## BN-17 - Customer Accounts And Reservations

- `burd-protocol` defines customer users, organizations, memberships, projects, quotas, customer API keys, credit ledger entries, marketplace reservations, usage summaries, and customer audit records.
- PostgreSQL migration `0016_customer_accounts_reservations` adds customer/account/project tables, hashed API keys, append-only customer credit ledger entries, marketplace reservations, and customer audit events.
- The control plane exposes admin endpoints under `/v1/customer/...` to create users, organizations, projects, quotas, API keys, credit entries, and audit-log reads.
- Customer API keys authenticate project reservation creation/listing, usage views, and reservation cancellation. Keys are stored as hashes and returned in plaintext only once.
- Reservation creation is idempotent, checks customer scope, project/organization status, project quota, listing status/current status, and optional workload-type binding. Active reservations are unique per marketplace listing.
- Customer credits are non-settlement accounting entries in BN-17. Reservation hold/release entries use zero credit movement because listing pricing and billing are BN-18 work.
- BN-17 does not implement checkout, job submission from reservations, provider-side execution, provider-set pricing, billing, Pix, payouts, invoices, refunds, disputes, taxes, or financial settlement.

## BN-18 - Billing, Pix And Payouts

- `burd-protocol` defines marketplace price records, Pix payment intents, financial ledger lines, billing invoices, balances, payout accounts, payouts, refunds, disputes, and reconciliation event contracts.
- PostgreSQL migration `0017_billing_pix_payouts` adds marketplace price book, Pix intents, billing invoices, append-only `financial_ledger_lines`, provider payout accounts, provider payouts, refunds, disputes, and reconciliation events.
- The control plane exposes admin endpoints for listing price, billing settlement, invoice reads, provider balances/ledger, payout account upsert, and payout creation.
- Customer API keys now support `billing:read` and `billing:write`; customer endpoints can create Pix payment intents and read project balances/ledger.
- Pix payment intents do not move money until confirmed by admin/adapter; confirmation appends balanced ledger lines.
- Reservation billing settlement requires BN-15 usage, BN-17 reservation, matching provider/device/GPU binding, an active BN-18 listing price, and sufficient confirmed project balance.
- Provider payouts require verified KYC/tax state, minimum payout, payable balance, and hold policy.
- BN-18 does not call a real Pix gateway, verify webhook signatures, execute bank payouts, provide checkout UI, or complete legal/KYC/tax workflows.

## BN-19 - Observability And SRE

- `crates/burd-control-plane` adds an `observability` module for in-memory HTTP metrics, recent normalized request events, background task error counters, SLO snapshots, and Prometheus text export.
- The HTTP router exposes `GET /metrics` publicly for aggregate operational metrics and `GET /v1/observability/snapshot` behind admin authorization.
- HTTP responses include `x-burd-correlation-id`, reusing a valid incoming `x-burd-correlation-id` or `x-request-id`, otherwise generating a backend request ID.
- The control plane emits structured JSON logs for service start, HTTP requests, background task errors, and PostgreSQL connection errors without logging credentials or raw request bodies.
- Config adds `BURD_CONTROL_DEPLOYMENT_ID`, `BURD_CONTROL_OBSERVABILITY_RECENT_EVENTS_LIMIT`, `BURD_CONTROL_SLO_AVAILABILITY_TARGET_BPS`, and `BURD_CONTROL_SLO_P95_LATENCY_MS`.
- `docs/bn-19-observability-sre.md` records the operational runbook, metrics contract, SLO contract, backup/restore expectation, and explicit non-goals.
- BN-19 does not implement OpenTelemetry Collector export, dashboards-as-code, alert routing, automated backup scheduling, automated restore tooling, incident ticket integration, or distributed tracing across services.
