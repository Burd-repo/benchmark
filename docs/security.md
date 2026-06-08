# Security

This MVP implements local security primitives for provider validation. It is not
production-grade platform security yet.

Implemented now:

- Ed25519 local keypair generation.
- Private key stored separately from `agent.json`.
- Reports never include the private key.
- Signed reports include canonical report hash, public key, signature, signing
  algorithm, timestamp, and local verification status.
- Challenge responses bind `challenge_id`, `nonce`, `provider_id`,
  `machine_id`, and `report_hash`.
- Challenges have `expires_at`, required minimum versions, required tests, and
  a local policy object.
- Local challenge verification checks nonce, expiry, minimum versions, required
  tests, report hash, signed report signature, and challenge response signature.
- API token creation and rotation store only `api_token_hash` in
  `~/.burd/agent.json`.
- Sensitive local API endpoints accept `Authorization: Bearer <token>` when
  `api_auth_enabled` is true.
- `127.0.0.1` can run in dev mode without a token, with an explicit warning.
- `0.0.0.0` emits a strong warning if no API token is configured.
- Raw data redacts private key paths and never reads private key material into
  the raw API response.
- Raw data includes `redacted: true` and `redacted_fields`.
- Actions and logs record important local operations.

Not implemented yet:

- Backend-side signature verification.
- Backend-issued challenges.
- mTLS or browser login for remote access.
- Hardware attestation.
- Fraud scoring beyond local heuristic warnings.
- Production key storage such as OS keychain, TPM, HSM, or encrypted secrets.

Recommended operation for this MVP:

1. Run the local API on `127.0.0.1`.
2. Generate identity before signed reports.
3. Create an API token before binding beyond loopback:
   `burd-agent api-token create --json`.
4. Treat `~/.burd/agent.key` as sensitive.
5. Use signed reports and challenge responses only as local validation artifacts
   until the Burd backend exists.
