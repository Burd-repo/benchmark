# Evidence Expiration

Burd evidence is valid for a limited period. Cryptographically valid evidence
can still be too old for readiness, provider verification, registration, or a
future marketplace decision.

## Local MVP Policy

| Evidence | TTL |
| --- | ---: |
| Full benchmark report | 7 days |
| Signed report | 7 days |
| Challenge and challenge response | 24 hours |
| Readiness | Derived from current report and challenge status |
| Provider session | Expires at the stored `expires_at` and is re-evaluated on `session status` or heartbeat |
| Capability spot / spot verification | Local/mock signal derived from current evidence; future remote spot checks should use short TTLs |

## Freshness Contract

Evidence freshness uses:

```json
{
  "issued_at": "2026-06-10T00:00:00+00:00",
  "expires_at": "2026-06-17T00:00:00+00:00",
  "is_expired": false,
  "age_seconds": 0,
  "ttl_seconds": 604800
}
```

`is_expired` and `age_seconds` are point-in-time values. Verifiers recalculate
them from trusted issuance/expiry dates instead of trusting persisted flags.

Signed report freshness is derived from `signed_at` plus the local signed-report
TTL. The informative envelope freshness is outside the canonical signed report
hash; changing it cannot extend validity because verification recalculates it.

## Readiness States

Readiness exposes separate evidence states:

- `missing`: no persisted evidence exists;
- `invalid`: evidence exists but parsing, hash, signature, or policy validation
  failed;
- `expired`: evidence is structurally/cryptographically valid but too old;
- `valid`: evidence is valid and unexpired.

An expired report or challenge receives no readiness points and produces a
renewal recommendation. Expiration alone is not treated as signature tampering
or a critical redaction failure.

## Verification And Registration

Provider verification exposes:

- `signed_report_evidence`;
- `signed_report_current`;
- `challenge_evidence`;
- warnings and failed checks such as `signed_report_expired` or
  `challenge_expired`.

Registration payloads include an `evidence` summary. An expired signed report
does not supply the signed benchmark score/tier used by registration; current
local provider values remain available for diagnostics.

## Session And Workload Impact

Expired signed reports, expired challenge evidence, fingerprint mismatch, or
expired sessions keep readiness, trust, and workload eligibility conservative.
They do not imply tampering by themselves, but they prevent old evidence from
being treated as current proof for future marketplace paths.

## Compatibility

Legacy JSON without freshness fields can still be parsed. When trusted issuance
dates exist, current freshness is derived during verification and loading.
