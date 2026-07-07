# ADR 0001 - Rust Modular Monolith Control Plane

## Status

Accepted for BN-00.

## Context

The local Provider Agent now produces hardware, fingerprint, signed evidence,
expiration, session, heartbeat, reliability, network, trust, capability,
performance, and workload eligibility signals. Those signals are useful local
evidence, but they are not network authority.

The next phase needs a backend that can enroll providers, issue challenges,
observe sessions, ingest telemetry, persist evidence, and calculate trust
without accepting provider self-attestation as final truth.

Starting with microservices, Kubernetes, billing, marketplace listings, jobs,
and scheduler orchestration at the same time would create too many trust and
operational boundaries before the remote contracts are stable.

## Decision

Burd Network starts as a Rust modular monolith named the Burd Control Plane.

The initial internal modules are:

- Provider Registry
- Session Service
- Verification Service
- Challenge Service
- Telemetry Ingestion
- Trust and Policy Engine
- Job Control, interface only until BN-13/BN-14
- Audit Log

The initial data dependencies are:

- PostgreSQL as the authoritative relational store
- object storage for complete signed envelopes, challenge responses, reports,
  telemetry windows, and future artifacts
- a simple queue abstraction for background work

The provider agent connects outbound to the control plane. The first remote
session protocol must not require the provider to expose a public inbound port.

## Consequences

- BN-01 can ship one deployable backend while preserving internal boundaries.
- Rust can reuse existing protocol, canonicalization, Ed25519, and evidence
  logic from this workspace.
- PostgreSQL becomes authoritative for provider, device, key, session, and
  evidence state.
- Object storage carries large immutable envelopes; PostgreSQL stores hashes,
  status, metadata, and pointers.
- Kafka, Kubernetes, distributed jobs, billing, Pix, payouts, marketplace UI,
  and multi-provider scheduling stay out of BN-01.

## Revisit

Revisit this decision after real remote sessions, recurring Proof of Capability,
jobs, leases, and metering exist. Operational scale, not anticipated scale,
should decide when to split services or add Kubernetes.