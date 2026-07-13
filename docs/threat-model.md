# Threat Model

This threat model covers the first Burd Network control-plane phase: provider
enrollment, remote sessions, signed evidence, challenge response, telemetry,
trust policy, audit logs, and BN-12 secure runtime planning.

It does not cover paid job execution, customer workload payloads, customer data
plane transfer, billing, Pix, payouts, Kubernetes, distributed training, or
marketplace UI.

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
- Private keys, API tokens, enrollment tokens, credentials, and raw secrets are
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
- future job secrets and artifact credentials, reserved for BN-13.

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
- future customer submitting approved workload templates, reserved for BN-13 and later.

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
| Private key exfiltration | Never transmit private key; future BN-20 adds TPM/HSM/OS keychain. |
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
| Wrong GPU used for workload | Runtime plan binds `--gpus device=<GPU UUID>`; future leases must bind provider, device, session, job, lease, and GPU UUID. |

## Antifraud Signals For BN-01 Through BN-09

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
- evidence hash collision or duplicate envelope with conflicting metadata.

## Privacy Boundaries

The backend should store what it needs to verify provider claims and operate the
network. It should not collect private files, local paths, API tokens, private
keys, arbitrary process arguments, wallet/payment data, or customer workload
payloads during BN-01.

Telemetry that can identify local activity should be minimized, redacted, and
retained according to explicit policy.

## Residual Risk

Without TPM/HSM/OS keychain support, a stolen private key can impersonate a
device until revoked. Without remote hardware attestation, the backend still
relies on signed observations plus consistency checks. Those risks are accepted
for BN-01 and revisited in BN-20.