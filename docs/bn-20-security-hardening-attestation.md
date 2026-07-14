# BN-20 - Security Hardening And Attestation

BN-20 adds the first backend-owned security hardening and attestation posture
registry. It does not make TPM, HSM, OS keychain migration, signed updater,
secret-manager integration, SBOM generation, vulnerability scanning, or remote
quote verification production-complete. It creates the signed contract and
control-plane authority needed to accept those signals without treating raw
agent claims as final trust.

## Scope

Implemented in this stage:

- protocol structs for a signed security posture payload;
- canonical posture hash and Ed25519 signature message binding provider, device,
  session, hardware fingerprint, public key, and posture hash;
- PostgreSQL `device_security_postures` registry with immutable rows;
- backend verification of posture hash, active provider key, signature, session
  binding, and hardware fingerprint binding;
- backend policy classification for release signature posture, key storage,
  attestation mode/evidence, SBOM hash, and scan status;
- admin endpoint for current security policy;
- device endpoint for submitting signed posture through an authenticated remote
  session;
- admin endpoint for listing provider security posture history;
- audit events for accepted and rejected posture submissions.

## API

### `GET /v1/security/policy`

Admin endpoint. Returns the active hardening policy version and configured
requirements:

- minimum agent version;
- whether signed release verification is required;
- whether hardware-backed non-exportable key storage is required;
- whether remote attestation evidence is required;
- whether SBOM hash is required;
- accepted release channels;
- accepted attestation modes.

### `POST /v1/sessions/{session_id}/security-posture`

Device endpoint. Requires the short-lived device bearer credential for the
remote session. The submitted payload is signed by an active provider device key.

The backend accepts the posture only when:

- `schema_version` and canonicalization version are supported;
- provider, device, and session IDs match the authenticated session;
- the session is currently `online` or `degraded`;
- the submitted hardware fingerprint matches the remote session fingerprint;
- `posture_hash` equals the canonical payload hash;
- `public_key_id` is active for that provider device;
- the Ed25519 signature verifies against the security posture signature message.

A valid posture can still be classified as `needs_hardening` when policy gates
are not satisfied. Invalid signature, inactive key, bad binding, or bad hash is
rejected and audited.

### `GET /v1/providers/{provider_id}/security-postures`

Admin endpoint. Lists the latest immutable posture records for a provider. The
response includes backend verification booleans, policy status, warnings, and
server receipt time.

## Policy Configuration

Environment variables:

```txt
BURD_CONTROL_SECURITY_MIN_AGENT_VERSION
BURD_CONTROL_SECURITY_REQUIRE_SIGNED_AGENT_RELEASE=false
BURD_CONTROL_SECURITY_REQUIRE_HARDWARE_BACKED_KEY=false
BURD_CONTROL_SECURITY_REQUIRE_REMOTE_ATTESTATION=false
BURD_CONTROL_SECURITY_REQUIRE_SBOM_HASH=false
BURD_CONTROL_SECURITY_ACCEPTED_RELEASE_CHANNELS=dev,stable
BURD_CONTROL_SECURITY_ACCEPTED_ATTESTATION_MODES=none,tpm,os_keychain,hsm,sev_snp,sgx
```

The defaults are intentionally permissive for early rollout. They record posture
and surface warnings without blocking existing providers before the agent has
production key storage, signed releases, and attestation integrations.

## Authority Rules

The agent may claim key storage backend, attestation mode, local quote status,
SBOM hash, binary hash, and scan status, but those fields are not final security
truth. The backend only attests that the posture was signed by the active device
key, bound to the remote session and fingerprint, and evaluated against the
current policy.

`server_received_at` is authoritative for registry timing. Provider timestamps
are recorded as observations only.

## Non-Goals

BN-20 does not implement:

- TPM quote parsing or verifier-specific attestation validation;
- HSM or OS keychain migration in the agent;
- production secret-manager integration;
- signed auto-update distribution;
- release signing infrastructure;
- SBOM generation or vulnerability scanner execution;
- RBAC model beyond existing admin/device/customer bearer scopes;
- external audit retention policy, penetration testing, or supply-chain scanning
  service integration.

These remain production-hardening follow-ups after the signed registry and
policy boundary are stable.