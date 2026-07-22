# API Security Contracts Hardening

## Summary

This hardening pass focused on the existing control-plane HTTP boundary. It did not add product features or new endpoints.

## Scope

- control-plane error envelopes
- bearer credential header parsing
- idempotency key header validation
- session control headers
- control-channel URL host reflection
- observability correlation and rate-limit header handling
- OpenAPI security and header contracts

## Issues Found

- Database errors were returned to clients as `database unavailable: {source_error}`, which could expose operational details if lower layers included connection or SQL context.
- `Idempotency-Key` accepted unbounded header values, allowing oversized keys to reach persistence paths.
- Bearer credentials and session headers accepted trimmed values, making whitespace-padded credentials ambiguous.
- The control-channel URL reflected the inbound `Host` header without shape validation.
- Correlation IDs and rate-limit keys could persist secret-like caller-supplied header values in in-memory observability state.
- OpenAPI documented `Idempotency-Key` as an unconstrained string.

## Changes Made

- Redacted client-facing database errors to a generic `database unavailable` envelope with `details.reason = database_unavailable`.
- Added strict visible-ASCII/no-whitespace bounds for bearer credentials, session headers, and `Idempotency-Key`.
- Added a 128-character `Idempotency-Key` limit.
- Sanitized the reflected control-channel host and falls back to `127.0.0.1:8080` when the host shape is unsafe.
- Normalized rate-limit keys to the first forwarded address and rejects secret-like values.
- Rejects secret-like request/correlation IDs from observability storage and generates a server request ID instead.
- Updated OpenAPI bearer descriptions and `Idempotency-Key` schema bounds.
- Updated remote protocol and BN-01 docs for the redaction/header contract.

## Tests Added

- Database error redaction unit coverage.
- Bearer, idempotency, and session header validation coverage.
- Control-channel host reflection coverage.
- Observability/rate-limit header redaction coverage.
- OpenAPI security-boundary and `Idempotency-Key` schema coverage.

## Notes

`GET /metrics`, `GET /health`, `GET /ready`, `GET /openapi.json`, enrollment start/proof, and provider read remain intentionally unauthenticated where the current contract already exposes them. This PR only tightens existing boundaries; it does not introduce account auth, gateway Pix, production API key management, or new product flows.

## Validation

Passed:

- `cargo fmt --all --check`
- `cargo test -p burd-control-plane`
- `cargo test --workspace`
- `cargo build --workspace`

Not run:

- PostgreSQL ignored tests. This pass does not change migrations, SQL persistence, transactions, or database-backed lifecycle behavior; the changed behavior is covered by unit/API/OpenAPI contract tests.