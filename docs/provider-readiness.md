# Provider Readiness

Provider Readiness is a read-only local assessment that consolidates the state
required to prepare a provider for future Burd verification. It does not
register the provider, contact a backend, create jobs, or imply marketplace
approval.

Run:

```sh
burd-agent readiness
burd-agent readiness --json
```

The plain command prints a human-readable summary. `--json` emits the stable
structured contract used by automation and the local Provider Console.

## Contract

The result contains:

- `status`: machine-readable readiness state;
- `readiness_score`: weighted local score from `0` to `100`;
- `readiness_level`: `Not Ready`, `Partial`, or `Ready Locally`;
- `checks`: individual weighted local checks;
- `warnings`: current gaps or failures;
- `recommendations`: actionable local next steps.

Each check includes an `id`, label, status, earned score, maximum score, and
message. Check status is `passed`, `warning`, or `failed`.

## Statuses

| Status | Meaning |
| --- | --- |
| `uninitialized` | No local provider identity exists. |
| `not_verified` | Identity exists, but there is no valid signed/self-verified provider state. |
| `partial` | Some verification requirements pass, but one or more readiness checks remain. |
| `ready_locally` | All local checks pass. This is not backend or marketplace approval. |
| `failed` | A critical local integrity or redaction check failed. |

## Checks And Weights

| Check | Weight | What it validates |
| --- | ---: | --- |
| Identity | 15 | Identity config and configured private signing key are available. |
| Signed report | 20 | Latest signed report passes local hash/signature verification. |
| Challenge | 15 | Local challenge evidence exists in verification, signed report, or history. |
| Provider verification | 20 | Hardware, benchmark, and signature are self-verified locally. |
| History | 10 | At least one benchmark history entry is persisted. |
| API token | 10 | Local API authentication is enabled with a configured token. |
| Raw redaction | 10 | Raw data declares and applies the required secret redaction contract. |

`ready_locally` requires all seven checks to pass and therefore has a score of
`100`. Warnings do not receive partial weight.

Provider verification preserves optional `vram_source` and `vram_confidence`
metadata. A real measurement is marked `detected`; llmfit known-GPU table and
unified-memory fallbacks are marked `estimated`; explicit overrides are marked
`provided`. Estimated VRAM is acceptable for MVP local readiness when capacity
is otherwise available, but it is not production hardware attestation. Future
marketplace policy should prioritize or require detected/high-confidence VRAM.

## Collection Behavior

The command does not mutate local state. It does not create identity, tokens,
reports, challenges, or history entries. When identity exists, it reuses the
existing provider aggregation to evaluate local provider verification and raw
redaction.

Tests evaluate the same readiness contract with deterministic internal fixtures
and isolated temporary `BURD_AGENT_HOME` / `BURD_AGENT_CONFIG` paths. They do
not depend on the user's real `~/.burd` state or real hardware detection.

## Provider Console

The local Provider Console reads `GET /api/v1/readiness` and shows readiness
score, level, status, checks, warnings, and recommendations in a dedicated
Readiness tab.

`Ready Locally` only means the local contracts pass. Backend verification,
auditing, marketplace listing, jobs, payouts, and billing remain out of scope.
