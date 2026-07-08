# BN-01 - Backend Foundation

BN-01 introduces the first real Burd Control Plane crate. It is a backend
foundation, not a provider marketplace and not a job runner.

## Scope

Implemented in `crates/burd-control-plane`:

- HTTP service with Axum.
- Environment-based configuration.
- PostgreSQL connection and SQL migrations.
- `GET /health` liveness endpoint.
- `GET /ready` readiness endpoint with database and migration checks.
- JSON error envelope aligned with BN-00 `remote-protocol-v1`.
- Required `Idempotency-Key` handling for mutating provider creation.
- In-memory per-client rate-limit guard.
- Audit event persistence for provider creation.
- Static `GET /openapi.json` contract.
- Initial provider registry persistence through `POST /v1/providers` and
  `GET /v1/providers/{provider_id}`.
- Unit tests plus an ignored PostgreSQL integration test that uses an isolated
  schema.

## Non-Goals

BN-01 does not implement:

- remote provider enrollment proof flow;
- device credentials;
- remote session control channel;
- backend-issued Proof of Capability;
- telemetry ingestion beyond the schema foundation;
- trust/policy calculation;
- scheduler, leases, jobs, containers, marketplace listings, billing, Pix, or
  payouts.

Those remain BN-02 and later.

## Runtime Configuration

Required:

```text
BURD_CONTROL_DATABASE_URL=postgres://user:password@host:5432/database
```

Optional:

```text
BURD_CONTROL_ENV=local
BURD_CONTROL_HOST=127.0.0.1
BURD_CONTROL_PORT=8080
BURD_CONTROL_DATABASE_SCHEMA=burd_control
BURD_CONTROL_RATE_LIMIT_PER_MINUTE=120
```

`DATABASE_URL` is accepted as a fallback when `BURD_CONTROL_DATABASE_URL` is not
set.

Run locally:

```powershell
$env:BURD_CONTROL_DATABASE_URL = "postgres://postgres:postgres@127.0.0.1:5432/burd"
cargo run -p burd-control-plane
```

## Initial Tables

The first migration creates:

- `users`
- `providers`
- `devices`
- `provider_identities`
- `provider_public_keys`
- `hardware_snapshots`
- `evidence_records`
- `provider_sessions`
- `audit_events`
- `idempotency_keys`
- `schema_migrations`

`idempotency_keys` and `schema_migrations` are support tables required by the
BN-01 backend foundation.

## Provider Registry Smoke Path

Create a provider:

```http
POST /v1/providers
Idempotency-Key: provider-create-001
Content-Type: application/json

{
  "display_name": "Example Provider"
}
```

Fetch it:

```http
GET /v1/providers/{provider_id}
```

The create path persists a provider row, writes an audit event, stores the
idempotency result, and returns a request ID.

## Validation

Default local tests do not require PostgreSQL:

```powershell
cargo test -p burd-control-plane
```

The isolated database integration test is ignored by default. To run it, provide
a PostgreSQL URL with permission to create/drop schemas:

```powershell
$env:BURD_CONTROL_TEST_DATABASE_URL = "postgres://postgres:postgres@127.0.0.1:5432/burd_test"
cargo test -p burd-control-plane -- --ignored migrates_and_persists_provider_with_isolated_schema
```