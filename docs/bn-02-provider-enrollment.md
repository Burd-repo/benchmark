# BN-02 - Remote Provider Enrollment And Identity

BN-02 turns a local Ed25519 identity into a backend-attested provider device
identity. It does not create a remote session or claim that the device is
online.

## Trust Flow

1. An administrator creates an unregistered provider.
2. The control plane returns a short-lived, one-time enrollment token.
3. The agent submits its public key, machine ID, hardware fingerprint, versions,
   and registration payload.
4. The backend consumes the token, stores the public key as untrusted, and
   issues a short-lived nonce.
5. The agent signs canonical `burd.enrollment-proof.v1` claims.
6. The backend verifies Ed25519 possession and creates the device, identity,
   active public key, audit event, and short-lived credential transactionally.
7. The provider moves to `pending_verification`. BN-06 and BN-07 are responsible
   for capability verification.

The private key and raw device credential are never stored by the backend. Only
SHA-256 token and credential hashes are persisted.

## Control Plane Configuration

Required:

```text
BURD_CONTROL_DATABASE_URL=postgres://user:password@host:5432/database
BURD_CONTROL_ADMIN_TOKEN=<bootstrap-admin-secret>
```

Optional TTL policy:

```text
BURD_CONTROL_ENROLLMENT_TOKEN_TTL_SECONDS=600
BURD_CONTROL_ENROLLMENT_PROOF_TTL_SECONDS=300
BURD_CONTROL_DEVICE_CREDENTIAL_TTL_SECONDS=900
```

`BURD_CONTROL_ADMIN_TOKEN` protects provider creation, enrollment-token
issuance, device listing, and device revocation. The process stores its SHA-256
hash in configuration memory and never logs the raw value. This bootstrap model
is replaced by account/RBAC work in later phases.

## Enrollment API

- `POST /v1/providers` creates an `unregistered` provider and requires the admin
  bearer plus `Idempotency-Key`.
- `POST /v1/providers/{provider_id}/enrollment-tokens` returns one token once.
  Issuing a new token revokes any previous unused token for that provider.
- `POST /v1/enrollments` consumes the token and returns an enrollment nonce.
  Retrying the identical request while proof is pending returns the same nonce.
- `POST /v1/enrollments/{enrollment_id}/proof` verifies possession and returns
  `provider_id`, `device_id`, `public_key_id`, and a short-lived credential.
- `POST /v1/devices/{device_id}/credentials` rotates the device credential and
  immediately revokes the previous credential.
- `POST /v1/devices/{device_id}/key-rotations` issues a key-rotation nonce.
- `POST /v1/devices/{device_id}/key-rotations/{rotation_id}/proof` verifies the
  new key, activates it, and revokes the previous key atomically.
- `GET /v1/providers/{provider_id}/devices` lists registered devices without
  secret material.
- `POST /v1/devices/{device_id}/revoke` revokes the device, active keys,
  credentials, identities, and pending rotations.

## Agent Flow

Initialize the local identity, place the one-time token in the environment, and
run enrollment:

```powershell
burd-agent identity init
$env:BURD_ENROLLMENT_TOKEN = "<one-time-token>"
burd-agent enrollment enroll --control-plane-url http://127.0.0.1:8080
Remove-Item Env:BURD_ENROLLMENT_TOKEN
```

Inspect redacted status or rotate the short-lived credential:

```powershell
burd-agent enrollment status --json
burd-agent enrollment refresh-credential --json
```

The agent stores the remote binding and raw credential separately at
`~/.burd/remote-enrollment.json`. Status and action logs expose IDs and expiry,
not the credential. Local-only `identity rotate-key` is blocked after remote
enrollment because changing only the local key would desynchronize the device.
Remote key rotation is available through the control-plane API.

## Replay And Revocation Rules

- enrollment tokens are short-lived and one-time;
- proof nonces are bound to enrollment, provider, machine, public key,
  fingerprint, and server expiry;
- server time decides expiration;
- completed proof nonces cannot be reused;
- active public keys cannot be shared by devices;
- a provider/machine pair cannot be enrolled twice;
- credential refresh revokes the previous credential in the same transaction;
- key rotation proves possession of the new key before revoking the old key;
- device revocation cascades to identities, keys, credentials, and pending
  rotations.

## Non-Goals

BN-02 does not implement:

- WebSocket or gRPC control channels;
- remote online/offline state;
- heartbeat loops or session resume;
- telemetry ingestion;
- Proof of Capability;
- trust calculation, jobs, scheduler, marketplace, billing, or payouts.

Those remain BN-03 and later.

## Validation

Default tests:

```powershell
cargo test -p burd-protocol -p burd-control-plane -p burd-agent
```

PostgreSQL integration tests use isolated schemas and run in CI:

```powershell
$env:BURD_CONTROL_TEST_DATABASE_URL = "postgres://postgres:postgres@127.0.0.1:5432/burd_test"
cargo test -p burd-control-plane -- --ignored
```
