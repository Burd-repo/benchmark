# Trust Score

Trust Score is the local heuristic confidence signal for a provider. It answers:

> How trustworthy is this provider's current local evidence and recent history?

It is not backend approval, not marketplace admission, not reputation ranking,
and not payout eligibility.

## Commands And API

```sh
burd-agent trust-score --json
```

```txt
GET /api/v1/trust-score
```

## Inputs

Trust Score combines existing local signals:

- provider verification integrity;
- signed report and challenge freshness;
- hardware fingerprint match state;
- local reliability score and heartbeat history;
- local network score;
- benchmark history depth;
- optional provider session status.

## Contract

The report contains:

- `trust_score`: score from `0` to `100`;
- `level`: human-readable trust level;
- `status`: machine-readable status;
- `components`:
  - `verification_integrity`;
  - `evidence_freshness`;
  - `reliability`;
  - `network`;
  - `history_depth`;
- `verification`: signed report, challenge, fingerprint, audit, and fraud-risk
  summary;
- `history`: benchmark-history summary;
- optional `session`: local session status, local online flag, and heartbeat
  count;
- `warnings` and `notes`.

## Scoring Model

The local MVP combines:

```txt
40% verification integrity
20% evidence freshness
20% reliability
10% network
10% benchmark history depth
```

High fraud risk or fingerprint mismatch can sharply reduce trust and produce
blocking warnings. Missing heartbeat history, expired reports, expired
challenge evidence, empty benchmark history, and weak network status keep trust
conservative.

## Product Semantics

Trust is historical confidence, not raw power.

- Compute Score answers how capable the machine is.
- Network Score answers which network-sensitive workloads fit.
- Reliability answers whether local availability has been stable.
- Trust Score summarizes whether the provider evidence is coherent enough to be
  considered for future paid paths.

## Marketplace Boundary

Future marketplace policy may require minimum trust before showing a provider or
routing paid workloads. The local MVP only calculates a deterministic local
heuristic. It does not contact a backend, calculate global reputation, create a
lease, schedule a job, or approve payouts.