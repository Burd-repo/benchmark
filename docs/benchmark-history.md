# Benchmark History

Benchmark history is local and persisted at:

```sh
~/.burd/benchmark-history.json
```

Use `BURD_AGENT_HOME` to place this file somewhere else for automation or
tests.

## Commands

```sh
burd-agent history --json
burd-agent history latest --json
burd-agent history export --output history.json
burd-agent history clear --confirm
```

`report --run-all` appends an unsigned history entry. `report --run-all
--signed` appends a signed history entry. Challenge runs also append signed
history entries with the challenge id.

## Entry Fields

- `history_id`
- `timestamp`
- `agent_version`
- `benchmark_version`
- `provider_id`
- `machine_id`
- `benchmark_profile`
- `system_summary`
- `gpu_summary`
- `score`
- `tier`
- `llm_benchmark_summary`
- `stability_summary`
- `network_summary`
- `disk_summary`
- `report_hash`
- `signed`
- `challenge_id`
- `verification_status`
- `warnings`

The history file stores summaries, not private key material, API tokens, or full
raw reports.

If the history file is missing, commands return an empty list. If it exists but
contains invalid JSON, the CLI returns a clear error instead of silently
discarding the file.
