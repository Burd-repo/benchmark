# Provider Identity

`burd-agent identity init` creates the local provider identity at
`~/.burd/agent.json` by default.

The config contains public provider metadata:

- `provider_id`
- `machine_id`
- `api_url`
- `preferred_provider`
- `benchmark_profile`
- `telemetry_enabled`
- `created_at`
- `public_key`
- `key_algorithm`
- optional contact/location fields

The private key is stored separately at `~/.burd/agent.key` by default. Reports,
provider details, raw data, and API responses must not expose this private key.

Commands:

```sh
burd-agent identity init
burd-agent identity show --json
burd-agent identity rotate-key --confirm
```

For tests or automation, override paths:

```sh
set BURD_AGENT_HOME=C:\path\to\state
set BURD_AGENT_CONFIG=C:\path\to\state\agent.json
```

Key rotation preserves `provider_id` and `machine_id`, but replaces the local
Ed25519 signing key and public key. Any old signed reports remain verifiable only
with their embedded public key.

