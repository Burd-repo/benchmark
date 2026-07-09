# BN-05 - Remote Evidence Registry

BN-05 moves signed local evidence into a backend-authoritative registry. The
agent still produces signed reports, but the control plane now verifies,
indexes, stores, expires, deduplicates, and revokes those records.

## Scope

Implemented in BN-05:

- session-authenticated evidence submission;
- canonical hash verification for `SignedReport`;
- Ed25519 verification against the active backend device key;
- provider/device/machine/fingerprint binding;
- server-side freshness and expiration calculation;
- immutable object storage for complete signed envelopes;
- PostgreSQL metadata, hash, verification state, and object pointer;
- global deduplication by evidence hash;
- admin list/read/revoke endpoints;
- audit events for accepted, rejected, and revoked evidence;
- hardware snapshot persistence from accepted signed reports.

Not implemented in BN-05:

- backend-issued Proof of Capability;
- challenge issuance and nonce lifecycle;
- benchmark profile v2;
- trust/antifraud scoring;
- jobs, scheduler, leases, billing, Pix, payouts, or marketplace listings.

## API

Provider devices submit evidence through their remote session:

```text
POST /v1/sessions/{session_id}/evidence-records
```

Required headers are the same session headers used by heartbeat and telemetry:

```text
Authorization: Bearer <device credential>
X-Burd-Session-Token: <session resume token>
X-Burd-Device-Id: <device_id>
```

The initial request accepts a signed report envelope:

```json
{
  "evidence_type": "signed_report",
  "session_id": "session_...",
  "subject_id": null,
  "metadata": null,
  "signed_report": { "...": "SignedReport" }
}
```

Admin endpoints:

```text
GET  /v1/providers/{provider_id}/evidence-records?limit=50
GET  /v1/evidence-records/{evidence_id}
POST /v1/evidence-records/{evidence_id}/revoke
```

Revocation stores `status=revoked`, `revoked_at`, `revocation_reason`, and an
audit event. The object-storage envelope is not deleted by revocation.

## Server Verification

The backend recalculates and stores verification state. It does not accept these
fields as authoritative when supplied by the agent:

- `signature_valid_locally`;
- `evidence.is_expired`;
- `report.evidence.is_expired`;
- local trust, readiness, score, or eligibility flags.

For `SignedReport`, the backend checks:

1. canonicalization version is `burd-json-c14n-v1`;
2. canonical report hash equals `signed_report.report_hash`;
3. envelope hash is computed server-side;
4. key algorithm is Ed25519;
5. report public key is the active backend key for the enrolled device;
6. signature verifies against the active backend key;
7. signed provider ID matches the backend provider ID or enrolled local provider ID;
8. signed machine ID matches the enrolled device machine ID;
9. signed hardware fingerprint matches the remote session fingerprint;
10. enrolled fingerprint, when present, matches the signed fingerprint;
11. freshness is recalculated from `signed_at` and server time.

Expired but otherwise valid evidence is stored with `status=expired`. Invalid
hashes, signatures, key bindings, provider bindings, device bindings, or
fingerprints are rejected and audited.

## Storage

PostgreSQL stores evidence metadata in `evidence_records`:

- `evidence_id`;
- `provider_id`, `device_id`, `session_id`;
- `evidence_type`, `subject_id`;
- `canonicalization_version`;
- `evidence_hash`;
- `report_hash`;
- `hardware_fingerprint`;
- `public_key_id`;
- `object_key`;
- `status`;
- `server_received_at`, `issued_at`, `expires_at`;
- `revoked_at`, `revocation_reason`;
- `verification_json`.

Complete signed envelopes are written to object storage under:

```text
evidence/{provider_id}/{evidence_hash}.json
```

For local/dev deployments this object store is filesystem-backed and controlled
by:

```text
BURD_CONTROL_OBJECT_STORAGE_DIR=./.burd-control-objects
```

Production can replace the backing store later without changing the registry
metadata contract.

## Deduplication

`evidence_hash` has a unique index. Re-submitting the same signed envelope
returns the existing registry record with `duplicate=true` instead of creating a
second record.

## Deferred

BN-06 introduces active backend-issued Proof of Capability. BN-07 will make
verification recurring and risk-based. BN-09 will consume accepted evidence,
telemetry, session history, and proof challenge history for global trust and
antifraud.