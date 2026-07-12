# BN-10 - Benchmark Profiles v2

BN-10 creates the first backend-owned benchmark profile registry and signed benchmark result history. It does not run workloads on the agent yet; it defines and persists the contracts that BN-11 policy, BN-12 runtime, BN-14 scheduler, and future marketplace ranking can consume.

## Implemented Scope

- `burd-protocol` defines versioned benchmark profile requests, profile records, signed benchmark result payloads, result verification records, and list/submit responses.
- PostgreSQL migration `0010_benchmark_profiles_v2` adds `benchmark_profiles` and `benchmark_results`.
- The control plane exposes admin endpoints to upsert/list benchmark profiles and session-authenticated endpoints to submit signed benchmark results.
- Benchmark results bind provider, device, session, run ID, profile ID/version, backend, hardware fingerprint, GPU UUID, image digest, optional model/artifact hashes, profile parameters, timings, metrics, telemetry-window hash, active public key ID, canonical result hash, and Ed25519 signature.
- Backend verification recalculates the canonical result hash, verifies the active device key signature, checks remote-session binding and fingerprint, checks active profile binding, validates digest/timestamp/metric ranges, and stores threshold pass/fail state.
- Duplicate `result_hash` submissions return the existing result. Reusing `(provider_id, device_id, run_id)` with a different result hash is rejected.
- Profile parameters, result parameters, warnings, and descriptions are rejected when they look like secrets.
- Audit events are emitted for profile upserts and newly accepted benchmark results.

## API

### `POST /v1/benchmark-profiles`

Admin endpoint. Creates or updates one benchmark profile version.

Request fields:

- `profile_id`
- `profile_version`
- `workload_type`
- `display_name`
- optional `description`
- `image_digest`
- optional `model_hash`
- optional `artifact_hash`
- `required_backend`
- `min_vram_gb`
- `parameters`, redacted JSON object
- `warmup_seconds`
- `duration_seconds`
- `sample_count`
- `thresholds`
- optional `status: active | deprecated | disabled`

Thresholds currently include minimum tokens/s, sustained tokens/s, requests/s, maximum TTFT, maximum p95 latency, and maximum error rate.

### `GET /v1/benchmark-profiles`

Admin endpoint. Lists profile registry records ordered by workload and profile version.

### `POST /v1/sessions/{session_id}/benchmark-results`

Device endpoint authenticated with the remote-session headers. Submits one `SignedBenchmarkResult` for the current session.

The signed payload contains:

- `schema_version: burd-benchmark-result-v1`
- `provider_id`, `device_id`, `session_id`, and `run_id`
- `profile_id`, `profile_version`, `workload_type`, and `backend`
- `hardware_fingerprint` and `gpu_uuid`
- `image_digest`, optional `model_hash`, optional `artifact_hash`
- redacted benchmark `parameters`
- warmup, duration, sample count, start, and completion timestamps
- driver/CUDA versions
- measured metrics such as tokens/s, sustained tokens/s, requests/s, concurrency, TTFT, latency p50/p95/p99, performance/watt, energy, VRAM pressure, utilization, temperature, power, throttling, and error rate
- optional `telemetry_window_hash`
- redacted warnings

The envelope contains:

- `payload`
- `result_hash`, the canonical hash of `payload`
- `public_key_id`
- `signature`
- `canonicalization_version: burd-json-c14n-v1`

The signature message uses domain `burd.benchmark-result.v1` and binds result hash, provider, device, session, run, profile, fingerprint, GPU UUID, image digest, and public key ID.

Backend behavior:

- authenticates the device credential and session token;
- requires an `online` or `degraded` remote session;
- requires the session fingerprint to match the result fingerprint;
- loads the active benchmark profile by profile ID/version;
- recalculates the result hash;
- verifies Ed25519 with the active backend device key;
- checks profile, workload, backend, image digest, optional model/artifact hash, profile parameters, timing/sample configuration, and session binding;
- validates metrics and timestamps;
- stores the result as `succeeded` when thresholds are satisfied or `failed` when the signed result is valid but below profile thresholds.

### `GET /v1/providers/{provider_id}/benchmark-results`

Admin endpoint. Lists recent backend-verified benchmark results for a provider. The optional `limit` query parameter is clamped to `1..200`.

## Authority Rules

- The provider signs measurements, but the backend decides whether the result hash, signature, session, profile, backend, fingerprint, image, model, artifact, profile configuration, and thresholds are valid.
- A benchmark result below profile thresholds is stored as failed instead of being treated as fraud by itself.
- Local AI performance, local benchmark history, fit estimates, and local score remain diagnostic unless submitted through this signed remote contract and accepted by the backend.
- Profile registry state is backend-owned. Providers cannot self-define the profile that makes them eligible.

## Not Implemented Yet

- Agent-side runners for these backend profiles.
- Versioned container images or image execution.
- Long soak tests, image generation, embeddings, Whisper, file processing, or training-light runners.
- Automatic submission from `burd-agent` after benchmark execution.
- Scheduler or policy consumption of benchmark results.
- Marketplace ranking, listings, reservations, jobs, leases, metering, billing, Pix, payouts, or disputes.