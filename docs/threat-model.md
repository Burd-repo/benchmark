# Threat Model

This threat model covers the first Burd Network control-plane phase: provider
enrollment, remote sessions, signed evidence, challenge response, telemetry,
trust policy, audit logs, BN-12 secure runtime planning, BN-13 job control metadata, BN-14 scheduler leases, BN-15 usage ledger receipts, BN-16 marketplace listing registry snapshots, BN-17 customer accounts/reservations, BN-18 billing/Pix/payout settlement primitives, BN-19 observability/SRE primitives, and BN-20 security posture/attestation registry primitives.

It does not cover paid job execution, raw customer workload payload bytes, customer data
plane byte transfer, real Pix gateway integration, signed payment webhooks, executed bank payouts, completed KYC/tax/legal workflows, Kubernetes, distributed training, or
marketplace UI beyond backend listing/reservation/billing registry, vendor-specific telemetry export, alert routing, automated backup/restore tooling, production TPM/HSM/OS keychain migration, TPM quote verification, signed updater infrastructure, SBOM generation, vulnerability scanner execution, or external supply-chain scanning.

## Security Goals

- A provider cannot become remotely verified without proving possession of its
  enrolled Ed25519 private key.
- A provider cannot decide its own global trust, remote online state,
  marketplace eligibility, evidence freshness, or challenge result.
- Stale, replayed, tampered, or revoked evidence is not accepted as current
  proof.
- Backend-issued nonces are one-time, short-lived, and bound to specific proof
  requirements.
- Remote sessions are authenticated, sequenced, revocable, and expire on server
  policy.
- Audit history records every backend authority decision needed for later
  dispute, antifraud, and incident review.
- Operational logs, metrics, and snapshots must support incident review without becoming authority for provider trust or billing state.
- Security posture records must be signed by the active provider key, bound to an authenticated session and hardware fingerprint, and evaluated by backend policy before they affect hardening state.
- Private keys, API tokens, enrollment tokens, credentials, customer API keys, and raw secrets are
  not exposed in reports, raw payloads, logs, or object storage metadata.

## Assets

- provider private key on the provider machine;
- provider public key registry;
- enrollment token;
- short-lived session credential;
- provider, device, and session IDs;
- signed report envelopes;
- challenge requests and responses;
- telemetry batches and hashes;
- hardware fingerprints;
- GPU UUID and capability evidence;
- trust, reliability, network, and policy scores;
- PostgreSQL records;
- object-storage envelopes;
- audit log;
- runtime image digests and allowlists;
- secure runtime plans;
- job-scoped data-plane credentials, introduced as metadata in BN-13 and still requiring later byte-transfer enforcement;
- scheduler leases, lease status, lease expiry, and active GPU reservations;
- usage ledger entries, receipt hashes, source hashes, and metering quantities;
- customer organizations, projects, API key hashes, quotas, reservations, customer credit ledger entries, and customer audit events;
- marketplace price book records, Pix payment intents, billing invoices, append-only financial ledger lines, provider payout accounts, provider payouts, reconciliation placeholders, KYC/tax status, hashed Pix key material, and masked Pix key suffixes;
- operational logs, correlation IDs, aggregate metrics, SLO status, and observability snapshots;
- signed security posture payloads, posture hashes, release/key-storage/attestation metadata, SBOM/binary hashes, scan statuses, and backend verification records.

## Actors

- honest provider running the official agent;
- provider with misconfigured hardware or stale evidence;
- malicious provider trying to inflate score, fake hardware, replay proofs, or
  appear online without capacity;
- attacker with network access but no provider private key;
- attacker with stolen enrollment token;
- attacker with stolen provider private key;
- compromised or outdated agent binary;
- backend operator or automation with elevated access;
- customer/admin submitting approved workload templates through BN-13 job metadata, triggering BN-14 scheduler passes, inspecting BN-15 usage receipts, inspecting BN-16 marketplace listings, reserving BN-17 customer inventory, performing BN-18 billing or payout actions, inspecting BN-19 operational snapshots, and inspecting BN-20 security posture records.
- backend operator or SRE using logs, metrics, snapshots, readiness, and audit events during incident response.

## Trust Boundaries

```text
Provider host
  | local files, private key, hardware, telemetry
  v
Authenticated outbound channel
  | TLS + session credential + Ed25519 proof where required
  v
Burd Control Plane
  | verifier, registry, policy, audit
  v
PostgreSQL and object storage
```

Provider host data is untrusted until verified. Signed host data is evidence,
not final authority. Backend state is authoritative only when it is derived from
server-issued IDs, server time, verified signatures, stored policies, accepted
evidence, and backend observations.

## Threats And Controls

| Threat | Control |
| --- | --- |
| Fake provider enrollment | Short-lived token plus nonce signed by local private key. |
| Enrollment token replay | One-time token use, expiration, audit event, idempotency key. |
| Key substitution during enrollment | Public key stored before nonce proof; proof must verify against that key. |
| Private key exfiltration | Never transmit private key; BN-20 records signed key-storage posture and can require hardware-backed non-exportable keys by policy, but actual TPM/HSM/OS keychain integration remains future. |
| Signed report tampering | Recalculate canonical hash and verify Ed25519 signature server-side. |
| Stale report reuse | Server recalculates expiration and tracks superseded/revoked evidence. |
| Challenge replay | One-time nonce, challenge state machine, response hash, signature binding. |
| Local clock manipulation | Server receipt time and server-issued expiry are authoritative. |
| Fake online state | Remote session is based on authenticated channel, heartbeat sequence, and server receipt time. |
| Heartbeat replay | Monotonic sequence numbers and duplicate/gap detection. |
| Duplicate device/session | One active session per provider/device; duplicate sessions become audit and antifraud signals. |
| Fake GPU by name | Require GPU UUID, PCI IDs, VRAM evidence, telemetry consistency, and challenge proof. |
| Inflated performance | Compare challenge metrics to hardware class, historical results, telemetry, and policy thresholds. |
| Region spoofing | Use regional probes and channel observations; declared region is only a claim. |
| Heartbeat without capacity | Correlate heartbeat with telemetry, GPU utilization, process use, and availability state. |
| Object storage tampering | Store envelope hash in PostgreSQL; verify hash before use. |
| Idempotency abuse | Scope idempotency keys by actor, endpoint, method, and body hash. |
| Rate-limit bypass | Per-provider, per-device, per-IP, and per-token rate limits. |
| Operator mistake | Append-only audit events and explicit admin actions for revocation/unblock. |
| Unpinned or unapproved runtime image | BN-12 requires digest-pinned image references and local allowlist before emitting Docker args; backend image policy becomes authoritative for jobs. |
| Arbitrary customer shell payload | BN-12 runtime plan does not accept commands or entrypoint overrides; BN-13 must use approved templates only. |
| Container escape surface | Planned defaults use read-only rootfs, non-root user, dropped capabilities, no-new-privileges, seccomp, PID/memory/CPU limits, no network, no IPC sharing, explicit tmpfs, and cleanup requirement. |
| Wrong GPU used for workload | Runtime plan binds `--gpus device=<GPU UUID>`; BN-13 job metadata and BN-14 leases bind provider, device, session, job, and GPU UUID before execution. |
| Job replay or duplicate creation | `POST /v1/jobs` requires `Idempotency-Key`; backend stores body hash and rejects conflicting replay. |
| Job assigned to wrong session | Provider pull is authorized through the remote session credential and BN-14 requires a non-expired lease matching provider, device, session, job, and GPU. |
| Double assignment of one job | Active leases are unique per job and provider `jobs/next` consumes only one offered lease transactionally. |
| Double reservation of one GPU | Active leases are unique per provider/device/GPU while in `offered`, `accepted`, `provisioning`, or `active`. |
| Provider disappears after lease offer | Offered leases have short server-side TTL and later scheduler passes expire stale offers. |
| Lease replay after expiry | `jobs/next` requires `status = offered` and `expires_at` later than server time before assignment. |
| Data-plane credential leakage through URLs | BN-13 returns scoped artifact paths separately from the opaque job credential; raw credentials are not embedded in URLs. |
| Duplicate or reordered job progress | Job events require a unique monotonically provided sequence per job; duplicate sequences are rejected. |
| Terminal result rewrite | BN-13 rejects result changes after a job reaches a terminal state. |
| Usage ledger tampering | BN-15 stores canonical receipt/source hashes and database triggers reject update/delete on usage ledger entries. |
| Duplicate usage finalization | `UNIQUE(job_id, entry_type)` makes finalize idempotent and returns the existing receipt. |
| Provider-inflated transfer bytes | BN-15 uses backend-recorded artifact metadata only; byte-level verification remains future data-plane hardening. |
| Customer API key replay or leakage | BN-17 stores only token hashes, scopes keys to projects, supports expiry, and uses bearer auth over authenticated transport. |
| Double reservation of one listing | BN-17 enforces a unique active reservation per marketplace listing and checks listing current status transactionally. |
| Reservation quota bypass | BN-17 locks project quota and active reservation state before accepting a reservation. |
| Customer credit ledger tampering | BN-17 customer credit ledger entries are append-only; updates/deletes are rejected by trigger. |
| Provider or customer-submitted billing total | BN-18 derives invoice totals from BN-15 usage, BN-17 reservation binding, and the active admin price book. |
| Financial ledger tampering | BN-18 financial ledger lines are append-only; updates/deletes are rejected by trigger and corrections require compensating entries. |
| Unbalanced financial transaction | BN-18 appends every financial transaction through a balance check before writing ledger lines. |
| Pix payment replay or fake confirmation | Payment-intent creation is idempotent, and balance changes occur only when backend/admin adapter confirmation appends balanced ledger lines. |
| Provider payout without policy clearance | Payout creation requires verified KYC/tax status, minimum payout, hold policy, and sufficient provider payable balance. |
| Raw Pix key leakage | BN-18 payout accounts require stored hash material and masked suffixes, not raw Pix keys. |
| Secret leakage through logs or snapshots | BN-19 logs operational metadata only and must not log bearer tokens, raw payloads, Pix keys, or customer workload bytes. |
| Metrics cardinality exhaustion | HTTP paths are normalized before recent-event snapshots and metrics stay aggregate. |
| Unauthorized operational snapshot access | `/v1/observability/snapshot` requires admin bearer authorization; `/metrics` exposes aggregate-only data. |
| Correlation ID spoofing or log injection | Incoming IDs are length-limited printable ASCII; invalid IDs are replaced by backend-generated request IDs. |
| Fake security hardening posture | BN-20 requires the posture to be signed by an active provider device key, bound to the authenticated session and matching hardware fingerprint, and hash-verified by canonical payload. |
| Agent claims unsupported release, key storage, attestation, or artifact integrity | BN-20 evaluates those fields against backend policy and classifies the posture as needs_hardening when requirements are not met. |
| Security posture replay | BN-20 stores unique posture hashes, binds records to provider/device/session/fingerprint, and returns duplicates without creating new authority. |
| Raw secret leakage through posture warnings | BN-20 rejects warnings that look unredacted and stores only hashes/status metadata for binary, SBOM, and attestation evidence. |

## Antifraud Signals For BN-01 Through BN-20

- same GPU UUID under multiple providers;
- same public key across unrelated devices;
- same hardware fingerprint across unrelated providers;
- impossible hardware/driver/CUDA combinations;
- performance far above hardware class;
- repeated identical telemetry samples;
- heartbeat without telemetry;
- GPU occupied while advertised as available;
- sudden region changes;
- challenge response after expiry;
- nonce reuse;
- signature mismatch;
- evidence hash collision or duplicate envelope with conflicting metadata;
- security posture signature mismatch, unsupported attestation mode, missing required SBOM hash, failed scan status, or downgrade from hardware-backed key posture to software-file posture.

## Privacy Boundaries

The backend should store what it needs to verify provider claims and operate the
network. It should not collect private files, local paths, API tokens, private
keys, arbitrary process arguments, raw Pix keys, bank account secrets, payment gateway secrets, bearer tokens, admin/customer API keys, or customer workload
payloads during this phase. BN-18 stores only hashed Pix key material, masked suffixes, payment intent metadata, invoices, and ledger lines needed for settlement. BN-19 logs and snapshots must stay limited to operational metadata, normalized paths, counters, and correlation IDs. BN-20 stores posture metadata, hashes, and scan statuses, not raw private keys, raw attestation quotes, raw SBOM documents, scanner reports, or secret-manager credentials.

Telemetry that can identify local activity should be minimized, redacted, and
retained according to explicit policy.

## Residual Risk

BN-20 can record and enforce hardening posture policy, but without completed
TPM/HSM/OS keychain support, a stolen private key can still impersonate a device
until revoked. Without remote quote verification, the backend still relies on
signed posture observations plus consistency checks. These risks remain accepted
for the first production-hardening slice and require follow-up implementation.