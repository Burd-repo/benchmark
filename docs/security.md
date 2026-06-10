# Security

This MVP implements local security primitives for provider validation. It is not
production-grade platform security yet.

Implemented now:

- Ed25519 local keypair generation.
- Private key stored separately from `agent.json`.
- Identity migration validates the signing key pair and creates a timestamped
  backup before rewriting or importing state.
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
- Contract tests verify that signed reports, challenge responses, registration
  payloads, benchmark history, config, and raw data do not expose private key
  material, API token values, or API token hashes. These tests run against
  isolated temporary agent homes instead of the real `~/.burd` directory.
- Real-hardware CI is manual-only, targets a dedicated ephemeral self-hosted
  runner, uses a protected environment, disables persisted checkout
  credentials, and isolates Burd state under `runner.temp`.
- VRAM reports distinguish real system/driver/device measurements (`detected`)
  from llmfit name-table or unified-memory heuristics (`estimated`) and explicit
  user overrides (`provided`).
- Real VRAM measurements are not overwritten by lower-confidence estimates.
- Migration backups are not included in reports or raw payloads, but may retain
  legacy secret fields from an old config.

Not implemented yet:

- Backend-side signature verification.
- Backend-issued challenges.
- mTLS or browser login for remote access.
- Hardware attestation.
- Fraud scoring beyond local heuristic warnings.
- Production key storage such as OS keychain, TPM, HSM, or encrypted secrets.
- Backend attestation of VRAM source/confidence and enforcement of marketplace
  policy for estimated or user-provided capacity.

Recommended operation for this MVP:

1. Run the local API on `127.0.0.1`.
2. Generate identity before signed reports.
3. Create an API token before binding beyond loopback:
   `burd-agent api-token create --json`.
4. Treat `~/.burd/agent.key` as sensitive.
5. Use signed reports and challenge responses only as local validation artifacts
   until the Burd backend exists.
6. Treat `estimated` or `provided` VRAM as local/MVP evidence, not production
   hardware attestation.
7. Review and securely remove migration backups after validating the migrated
   state.
