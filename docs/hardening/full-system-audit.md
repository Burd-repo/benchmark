# Full System Audit

## Summary

This hardening pass stayed within the existing Burd shape. It did not add new product features.

What changed:
- fixed a fail-open path in the local API auth check so a corrupted auth config now fails closed on protected endpoints;
- corrected stale or broken documentation in the README, architecture note, and provider trust-layer note;
- added a regression test for corrupted local API auth config;
- documented the audit scope and the current implemented-vs-planned boundary.

## Scope Audited

- Burd Agent CLI and local API
- Burd Control Plane backend
- workspace layout, docs, and contract boundaries
- authentication/redaction behavior in the local API
- current BN documentation and implemented-vs-planned boundary

## What Is Implemented For Real

- local agent commands, reports, fingerprints, benchmark history, runtime planning, readiness, trust, capability spot, and workload eligibility
- local API serving the benchmark UI and local contracts
- control plane backend with registry, enrollment, sessions, evidence, PoC, telemetry, trust, workload policy, jobs, scheduler leases, metering, marketplace, customer accounts, billing primitives, observability, security posture, and multi-GPU inventory
- PostgreSQL-backed persistence and migrations for the implemented BN slices

## What Is Still Local, Mocked, or Future

- remote verification is still incremental and backend-owned, not a final production trust boundary
- jobs, scheduler behavior, billing settlement, Pix, payouts, and marketplace enforcement remain bounded by the existing BN implementation state
- no new production features were introduced in this audit

## Bugs Found And Corrected

- local API protected routes could fall open if `load_identity()` failed while an auth config file existed but was invalid; now they fail closed
- README contained a broken BN-04 markdown link line; fixed
- provider trust-layer documentation still described higher-numbered BN slices as future work; updated to reflect that those docs now exist
- architecture note now mentions BN-21 multi-GPU inventory snapshots

## Overengineering Removed

- no large abstractions were removed in this pass
- the only code simplification was centralizing the local API auth requirement check into a small helper so the security behavior is explicit and shared

## Event Bus And Listeners

- no production event bus/listener plumbing was found in the audited paths
- nothing was removed here because there was no real event-driven indirection protecting a boundary

## Migrations And Database

- no schema migration was changed in this pass
- existing migration coverage stayed intact under the workspace test suite
- no ledger or reservation schema was altered

## Security Findings

- fixed a fail-open auth path in the local API
- protected endpoints continue to require bearer auth when auth is configured
- corrupted auth config now yields unauthorized rather than bypassing protection
- no secret material was intentionally exposed in the audited paths

## Performance Findings

- no new hot-path regression was introduced
- no heavy request-path refactor was needed for this audit
- existing bounded loops and request handlers were left intact

## Tests Added Or Adjusted

- added `protected_endpoint_fails_closed_when_auth_config_is_corrupted`
- kept the existing protected-endpoint auth contract tests intact

## Tests Executed

- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo build --workspace`

## Tests Not Yet Executed

- none

## Risks Remaining

- remote verification, scheduler policy, billing settlement, Pix, payouts, and full marketplace enforcement still depend on later product work
- this audit did not attempt a large architectural rewrite

## Deferred Items

- broader cleanup of older documentation references outside the directly audited paths
- any larger refactor should wait for a concrete bug or a measured boundary leak

## Next Hardening PRs Recommended

1. a follow-up doc sync for any remaining stale references outside the core README and trust docs
2. a narrow pass over any remaining fail-open or silent-fallback paths in local and control-plane auth/error handling
3. a targeted contract audit around idempotency and state transitions if a real bug surfaces there