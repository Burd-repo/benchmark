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
burd-agent identity migrate --confirm
burd-agent identity migrate --from C:\path\to\existing-state --confirm
burd-agent identity show --json
burd-agent identity rotate-key --confirm
```

For tests or automation, override paths:

```sh
set BURD_AGENT_HOME=C:\path\to\state
set BURD_AGENT_CONFIG=C:\path\to\state\agent.json
```

State resolution is deterministic: `BURD_AGENT_CONFIG` and its parent directory
take precedence, then `BURD_AGENT_HOME`, then `~/.burd`. If both environment
variables disagree, `BURD_AGENT_CONFIG` wins and readiness reports a warning.
This prevents identity, reports, history, and challenge evidence from silently
using different directories.

`identity migrate --confirm` normalizes or repairs the current state.
`identity migrate --from <directory> --confirm` imports a valid existing state.
Migration creates a timestamped backup first, validates the signing key pair,
removes legacy secret fields from active `agent.json`, and copies persisted
evidence. When repairing the current state, an unavailable or invalid signing
key is replaced while preserving valid provider and machine IDs. Review and
securely remove old backups after validating migration because a backup may
retain legacy secret material.

Key rotation preserves `provider_id` and `machine_id`, but replaces the local
Ed25519 signing key and public key. Any old signed reports remain verifiable only
with their embedded public key.
