# Reliability Score

Reliability Score is the local measure of whether a provider stays available and
stable once it starts a local provider session. It answers:

> Does this machine remain online locally and keep heartbeat evidence coherent?

It does not measure raw compute capacity, does not change Burd Compute Score,
does not contact a backend, and does not imply marketplace admission.

## Commands And API

```sh
burd-agent uptime --json
burd-agent reliability --json
```

```txt
GET /api/v1/uptime
GET /api/v1/reliability
```

Both reports are derived from local heartbeat history in the canonical Burd
state directory. Tests must use temporary `BURD_AGENT_HOME` or
`BURD_AGENT_CONFIG` paths and never the user's real `~/.burd` state.

## Inputs

Reliability consumes local one-shot heartbeat records from
`~/.burd/uptime.json` or the redirected state directory. A heartbeat is created
only by an explicit one-shot command:

```sh
burd-agent heartbeat --once --json
```

The command is intentionally not a daemon and does not run continuously.

## Uptime Contract

`uptime --json` reports:

- `uptime_1d`
- `uptime_7d`
- `uptime_30d`
- `uptime_score`
- `uptime_level`
- `last_online_at`
- `last_failed_check_at`
- `checks_total`
- `checks_failed`
- `current_status`

## Reliability Contract

`reliability --json` and `GET /api/v1/reliability` report:

- `reliability_score`: local score from `0` to `100`;
- `uptime_score`: the uptime subscore;
- `level`: human-readable local reliability level;
- `status`: machine-readable state such as `no_history`, `warming_up`,
  `reliable`, `degraded`, or `offline`;
- `components`: uptime ratios, sample coverage, latest-status score, and
  failure penalty;
- `uptime`: embedded uptime summary;
- `checks_total`, `checks_failed`, and `consecutive_failed_checks`;
- `warnings` and `notes`.

## Product Semantics

Reliability is availability history, not hardware quality.

- A powerful GPU with no heartbeat history can keep high compute potential but
  has low or warming-up reliability.
- A weak GPU with stable heartbeat history can be reliable but not powerful.
- Bad internet or missed heartbeats should limit workload eligibility and future
  marketplace availability without erasing the machine's compute score.

## Future Marketplace Use

A future registry or scheduler may use reliability to decide whether a provider
can receive online workloads. The local MVP only prepares the evidence. It does
not create leases, jobs, scheduler assignments, payouts, billing, Pix, or a real
marketplace listing.