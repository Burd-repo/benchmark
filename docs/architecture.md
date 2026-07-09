# Architecture

## Overview

The Burd Agent is a local Rust CLI and API server used by GPU providers before a
machine can be evaluated for the future Burd marketplace.

Flow:

1. Provider installs and runs `burd-agent`.
2. Agent detects hardware using llmfit-derived core logic.
3. Agent analyzes model fit and recommended workloads.
4. Agent optionally runs real benchmarks against Ollama, vLLM or MLX.
5. Agent runs stability, network and disk checks.
6. Agent calculates Burd Compute Score and tier.
7. Agent generates a JSON report.
8. Future backend issues a challenge and validates a signed report.

## Crates

- `burd-agent`: CLI entrypoint.
- `burd-hardware`: Burd-shaped system report around llmfit hardware detection.
- `burd-llmfit`: adapter around `llmfit-core`.
- `burd-bench`: LLM, stability, network, disk, profile, score and report logic.
- `burd-control-plane`: BN-01/BN-02 Rust backend for registry, enrollment,
  device identity, migrations, health, readiness, idempotency, audit, and OpenAPI.
- `burd-protocol`: identity, enrollment proof, challenge, signed report and report envelope types.
- `burd-api-local`: local HTTP API and static benchmark UI serving.

## Future Backend Flow

1. Burd backend creates a `Challenge`.
2. Agent receives challenge and required tests.
3. Agent runs benchmarks locally.
4. Agent signs `SignedReport`.
5. Backend validates signature, challenge freshness, hardware consistency,
   benchmark plausibility and antifraud signals.
6. Marketplace accepts, rejects or requests manual review.

## Burd Network Control Plane

BN-00 freezes the first remote backend boundary. BN-01 adds the backend
foundation. BN-02 adds remote provider enrollment, Ed25519 possession proof,
short-lived device credentials, key rotation, and revocation. The target is a
Rust modular monolith backed by PostgreSQL, object storage, and a simple queue.
The authenticated outbound control channel remains BN-03.

The backend is authoritative for registry state, server-side evidence
expiration, challenge issuance, nonce use, remote session state, revocation,
global trust, policy decisions, and audit history. Provider-sent scores,
regions, online flags, expiration flags, and eligibility flags are evidence or
claims only.

See:

- [`bn-00-architecture-freeze.md`](bn-00-architecture-freeze.md)
- [`bn-01-backend-foundation.md`](bn-01-backend-foundation.md)
- [`bn-02-provider-enrollment.md`](bn-02-provider-enrollment.md)
- [`remote-protocol-v1.md`](remote-protocol-v1.md)
- [`remote-authority-matrix.md`](remote-authority-matrix.md)
- [`threat-model.md`](threat-model.md)
- [`adr/0001-control-plane-modular-monolith.md`](adr/0001-control-plane-modular-monolith.md)

## Marketplace Readiness

This repository prepares the data model and local validation flow. It does not
implement listing, scheduling, remote customer jobs, billing, Pix, payments or
provider payouts.
