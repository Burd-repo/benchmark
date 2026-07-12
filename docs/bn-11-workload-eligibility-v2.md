# BN-11 - Remote Policy And Workload Eligibility v2

BN-11 moves workload eligibility from a local/provider heuristic into a backend-owned policy and evaluation layer.

It does not schedule jobs, reserve GPUs, create leases, list marketplace supply, or bill customers. It produces an authoritative backend state that later scheduler and marketplace code can consume.

## Scope

BN-11 adds:

- versioned workload policy contracts in `burd-protocol`;
- PostgreSQL tables for `workload_policies` and `provider_workload_eligibility`;
- admin endpoints to upsert/list policies, run a sweep, and inspect provider states;
- backend evaluation over trusted BN-03 through BN-10 state;
- persisted reason codes and audit events for every sweep update.

## API

```txt
POST /v1/workload-policies
GET  /v1/workload-policies
POST /v1/workload-eligibility/sweep
GET  /v1/providers/{provider_id}/workload-eligibility
```

All endpoints require the admin bearer credential. Device credentials cannot create policy or self-approve eligibility.

## Policy Inputs

A workload policy can require:

- canonical workload type;
- minimum trust score and maximum risk score;
- minimum backend reliability and remote network score;
- required verification status and recent proof freshness;
- required benchmark profile ID/version and max benchmark age;
- CUDA/backend binding through signed benchmark results;
- minimum tokens/s, sustained tokens/s, requests/s, max TTFT, and max p95 latency;
- minimum VRAM from backend-accepted telemetry;
- allowed regions from backend regional reachability.

Fields such as price and GPU family are present in the contract for policy evolution, but BN-11 does not pretend they are verified when the current control-plane tables do not yet contain a normalized authoritative source for them.

## Eligibility Status

The backend stores one of:

- `eligible`
- `limited`
- `ineligible`
- `verification_required`
- `temporarily_unavailable`
- `blocked`

The priority order is conservative. Blocked provider/device/trust state wins. Missing proof, stale proof, missing benchmark, stale benchmark, or unavailable attestation produces `verification_required`. Weak network or reliability produces `limited`. Offline or missing remote session produces `temporarily_unavailable` unless a higher-severity policy failure exists.

## Authority Boundary

The provider does not submit eligibility, ranking, approval, or marketplace admission.

The backend derives eligibility from:

- provider/device registry state;
- latest remote session state;
- provider verification state;
- global trust and antifraud state;
- regional network state;
- signed GPU telemetry;
- signed benchmark results;
- backend policy version.

## Non-Goals

BN-11 does not implement:

- scheduler enforcement;
- leases or reservations;
- provider runtime containers;
- jobs or customer data plane;
- marketplace listings;
- pricing, billing, Pix, payouts, or settlement;
- production autonomous background sweep scheduling.

Those remain BN-12 through BN-18 work.
