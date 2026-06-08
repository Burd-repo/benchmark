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
- Challenges have `expires_at` and required minimum versions.
- Raw data redacts private key paths and never reads private key material into
  the raw API response.
- Actions and logs record important local operations.
- Serving on `0.0.0.0` emits a warning because API token support is future work.

Not implemented yet:

- Backend-side signature verification.
- Backend-issued challenges.
- API token or mTLS for remote access.
- Hardware attestation.
- Fraud scoring beyond local heuristic warnings.
- Production key storage such as OS keychain, TPM, HSM, or encrypted secrets.

Recommended operation for this MVP:

1. Run the local API on `127.0.0.1`.
2. Generate identity before signed reports.
3. Treat `~/.burd/agent.key` as sensitive.
4. Use signed reports and challenge responses only as local validation artifacts
   until the Burd backend exists.

