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
- `provider_details`
- `latest_signed_report_hash`
- `latest_score`
- `latest_tier`
- `location`
- `contact`
- `capabilities`
- `pricing`
- `verification`
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
