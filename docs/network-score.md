# Network Score

Network Score is the local network-quality signal for a provider. It answers:

> Which workload shapes can this connection support?

It is intentionally separate from Burd Compute Score. Internet quality can limit
real-time or interactive workloads, but it must not erase the measured compute
capacity of the GPU.

## Commands And API

```sh
burd-agent bench network --json
burd-agent network-score --json
```

```txt
GET /api/v1/network-score
```

`bench network --json` collects a finite local sample and persists it as
`~/.burd/latest-network.json` unless the state directory is redirected.
`network-score --json` loads the latest finite sample or the network section of
the latest full report. It does not start a daemon, bind a public port, or prove
public reachability.

## Benchmark Inputs

The finite benchmark can include:

- endpoint;
- attempts;
- latency average/min/max aliases;
- jitter;
- successful and failed request counts;
- request loss percentage;
- status code;
- DNS timing;
- probe duration;
- warnings and errors.

## Score Contract

`network-score --json` reports:

- `network_score`: score from `0` to `100`;
- `level`: human-readable level such as `No Data`, `Poor`, `Constrained`,
  `Usable`, `Strong`, or `Excellent`;
- `status`: machine-readable state such as `no_benchmark`, `failed`,
  `constrained`, `usable`, `strong`, or `ready`;
- `source`: endpoint or source file used for the score;
- `components`: latency, jitter, loss, DNS, success rate, and raw observed
  values;
- optional `benchmark` payload;
- `warnings` and `notes`.

## Workload Meaning

Network quality should influence workload eligibility:

- low latency, low jitter, and low loss can support real-time LLM APIs and
  interactive notebooks;
- medium network quality can still support batch inference, embeddings, and
  offline processing;
- high packet loss, severe jitter, or failing endpoints should block online
  marketplace/session availability in future backend policy;
- low upload/download capacity should limit large file workloads when those
  measurements are available.

## Boundaries

The local score is not a public SLA, not backend availability, and not
marketplace approval. Future backend checks may add regional probes, public
reachability, and scheduler-specific network policies.