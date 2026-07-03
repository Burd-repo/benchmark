# Provider Registration Payload

The registration payload is a local JSON artifact for the future Burd backend.
It is not submitted automatically in this MVP.

Commands:

```sh
burd-agent registration-payload --json
burd-agent registration-payload --output registration.json
```

Fields:

- `provider_id`
- `machine_id`
- `public_key`
- `agent_version`
- `benchmark_version`
- `hardware_fingerprint`
- `marketplace_policy`
- `provider_details`
- `latest_signed_report_hash`
- `latest_score`
- `latest_tier`
- `location`
- `contact`
- `capabilities`
- `pricing`
- `verification`
- `reliability`
- `network`
- `evidence`
- `created_at`
- `secrets_included`

The payload does not include:

- Ed25519 private key material;
- API token;
- API token hash;
- local secret paths;
- billing, Pix, payout, or financial account data.

`secrets_included` must remain `false`.

Future backend work can POST this payload to Burd, attach backend verification,
and produce marketplace eligibility. That is intentionally out of scope for this
local agent stage.

The current payload carries the live hardware fingerprint and local
`nvidia_cuda_only_mvp` policy snapshot. Provider verification also exposes the
latest signed-report fingerprint and whether it matches current hardware.

`reliability` and `network` carry local-only scores for future backend review.
`evidence` includes signed-report and challenge freshness. An expired signed
report remains visible by hash for audit/history purposes, but its signed score
and tier do not count as current registration evidence.