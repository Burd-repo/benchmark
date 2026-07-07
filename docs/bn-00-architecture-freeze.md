# BN-00 - Architecture Freeze

BN-00 freezes the first remote architecture boundary for Burd Network. It does
not implement a backend runtime, database migrations, scheduler, jobs,
marketplace, billing, Pix, payouts, or a new visual interface.

The goal is to make BN-01 implementation mechanical: backend code should follow
these contracts instead of inventing trust rules endpoint by endpoint.

## Deliverables

- Architecture ADR for the Rust modular monolith control plane.
- Threat model for enrollment, evidence, sessions, challenges, telemetry, and
  trust calculation.
- Remote `/v1` protocol design.
- Provider, session, challenge, evidence, and job state machines.
- Authority matrix defining agent-claimed, backend-attested, backend-derived,
  and never-accepted fields.
- Corrections to local documentation where wording confused local facts with
  future remote authority.

## Frozen Direction

The first backend is the Burd Control Plane, a Rust modular monolith with
PostgreSQL, object storage, and a simple queue.

```text
Burd Agent
    |
    | authenticated outbound connection
    v
Burd Control Plane
    |-- Provider Registry
    |-- Session Service
    |-- Verification Service
    |-- Challenge Service
    |-- Telemetry Ingestion
    |-- Trust and Policy Engine
    |-- Job Control
    `-- Audit Log
          |
          |-- PostgreSQL
          |-- Object Storage
          `-- simple queue
```

The backend is authoritative for registry state, server time, evidence
freshness, challenge issuance, nonce use, session status, remote online/offline
state, revocation, global trust, policy decisions, and audit history.

The provider agent remains authoritative only for local private-key custody and
for producing signed local observations. Those observations are evidence, not
final network truth.

## Non-Goals

BN-00 and BN-01 must not include:

- scheduler placement;
- paid jobs;
- provider runtime sandboxing;
- marketplace listings;
- billing, Pix, payouts, credits, invoices, or settlement;
- Kubernetes;
- distributed training;
- multi-provider jobs;
- visual marketplace or dashboard work.

## BN-01 Gate

BN-01 can begin when the implementation follows these constraints:

- `/v1` APIs use the remote protocol contract.
- PostgreSQL owns provider, device, key, session, and evidence state.
- every mutating endpoint has an idempotency strategy.
- enrollment proves possession of the local Ed25519 private key without
  transmitting it.
- server-side expiration is recalculated from trusted server time.
- backend audit events are emitted for enrollment, key changes, session changes,
  challenge lifecycle events, evidence acceptance/rejection, revocation, and
  policy status changes.
- no provider-sent score, region, online flag, expiration flag, or eligibility
  flag is treated as authoritative.

## Reference Documents

- [`adr/0001-control-plane-modular-monolith.md`](adr/0001-control-plane-modular-monolith.md)
- [`threat-model.md`](threat-model.md)
- [`remote-protocol-v1.md`](remote-protocol-v1.md)
- [`remote-authority-matrix.md`](remote-authority-matrix.md)