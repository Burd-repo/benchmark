# Workload Eligibility

Workload Eligibility is the local decision layer that maps provider evidence to
workload-specific suitability. It answers:

> Which AI workloads can this provider support locally, diagnostically, or as a
> future marketplace candidate?

It is not a lease, not a scheduler assignment, not a paid job, and not
marketplace approval.

## Commands And API

```sh
burd-agent workload-eligibility --json
```

```txt
GET /api/v1/workload-eligibility
```

## Inputs

The local report combines:

- system hardware and backend detection;
- NVIDIA/CUDA marketplace GPU policy;
- model fit recommendations;
- Burd Compute Score;
- provider verification and hardware fingerprint state;
- reliability score;
- capability spot verification;
- trust score;
- benchmark history depth;
- current signed evidence state.

## Contract

The top-level report contains:

- `checked_at`;
- `provider_tier`;
- `local_status`;
- `marketplace_status_future`;
- `workloads`;
- `summary`;
- `warnings` and `notes`.

Each workload row contains:

- `workload`;
- `local_status`;
- `marketplace_status_future`;
- `confidence_level`;
- `capability_score`;
- `trust_score`;
- `summary`;
- `reasons`;
- `blockers`.

## Current Status Values

Local status can include:

- `eligible_locally`: local signals support the workload;
- `diagnostic_only`: useful for local diagnostics but not strong enough for paid
  paths;
- `not_recommended`: fit analysis explicitly marks it outside the current
  recommendation set;
- `blocked`: evidence, trust, capability, fraud risk, or fingerprint state
  blocks it;
- top-level `not_ready`: no workload is currently eligible or diagnostic.

Future marketplace status can include:

- `marketplace_candidate`: local evidence suggests this workload could be a
  future paid marketplace candidate;
- `marketplace_blocked`: marketplace policy or evidence quality blocks the
  future paid path.

## Product Rules

The decision layer keeps compute, network, reliability, trust, and policy
separate:

- good compute plus weak network should allow batch-style diagnostics while
  limiting realtime or interactive paths in future policy;
- low VRAM blocks large model workloads;
- non-NVIDIA, ROCm, Vulkan-only, Apple Silicon, Intel GPU, and CPU-only systems
  remain local diagnostic only for the paid marketplace MVP;
- NVIDIA without CUDA blocks marketplace eligibility;
- unreliable or estimated VRAM blocks premium marketplace paths;
- expired signed reports or missing challenge evidence keep eligibility
  conservative;
- fingerprint mismatch or high fraud risk blocks local workload eligibility;
- inactive or expired sessions block online availability in future backend
  policy.

## Workload Vocabulary

The current engine starts from local fit recommendations and canonical MVP
workloads such as LLM inference, agents, embeddings, batch inference, SDXL, and
fine-tuning. The product vocabulary for future scheduler/marketplace policy
should converge on:

- `llm_realtime_api`;
- `llm_batch_inference`;
- `embeddings`;
- `image_generation`;
- `comfyui_remote`;
- `interactive_notebook`;
- `whisper_transcription`;
- `file_processing`;
- `training_light`;
- `training_heavy`;
- `large_model_inference`.


## Remote BN-11 Eligibility

BN-11 adds a separate backend-owned eligibility layer in `crates/burd-control-plane`.

Control-plane endpoints:

```txt
POST /v1/workload-policies
GET /v1/workload-policies
POST /v1/workload-eligibility/sweep
GET /v1/providers/{provider_id}/workload-eligibility
```

Remote statuses are:

- `eligible`;
- `limited`;
- `ineligible`;
- `verification_required`;
- `temporarily_unavailable`;
- `blocked`.

Remote eligibility is derived by the backend from provider/device registry state, remote session state, verification state, global trust/risk/reliability state, regional network state, signed telemetry, signed benchmark results, and backend policy version. The provider never submits this state as a trusted claim.

The local `burd-agent workload-eligibility --json` output remains diagnostic and useful before enrollment, but it is not remote marketplace approval.
## Future Scheduler Use

A future scheduler should consume Workload Eligibility before routing jobs. The
first production path should be one job to one provider, then one job to
multiple GPUs on the same machine, and only later one job across multiple
providers. The local MVP does not implement any of those scheduler paths.